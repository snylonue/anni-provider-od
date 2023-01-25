pub mod info;

pub use anni_provider::{AnniProvider, ProviderError};
pub use onedrive_api;

use anni_provider::{AudioInfo, AudioResourceReader, Range, ResourceReader};
use dashmap::DashMap;
use onedrive_api::{resource::DriveItem, Auth, DriveLocation, ItemLocation, OneDrive, Permission};
use reqwest::{
    header::{CONTENT_RANGE, RANGE},
    redirect::Policy,
    Client, ClientBuilder,
};
use std::{
    borrow::Cow,
    collections::HashSet,
    num::NonZeroU8,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

pub struct OneDriveClient {
    drive: OneDrive,
    auth: Auth,
    refresh_token: String,
    client_secret: String,
    expire: Duration,
    client: Client,
    location: DriveLocation,
}

impl OneDriveClient {
    pub async fn new(
        refresh_token: String,
        client_id: String,
        client_secret: String,
        location: DriveLocation,
    ) -> Result<Self, Error> {
        let client = ClientBuilder::new()
            .redirect(Policy::none())
            .build()
            .unwrap();
        let auth = Auth::new_with_client(
            client.clone(),
            &client_id,
            Permission::new_read().offline_access(true),
            "",
        );
        let token = auth
            .login_with_refresh_token(&refresh_token, Some(&client_secret))
            .await?;
        let access_token = token.access_token;
        let refresh_token = token.refresh_token.expect("Fail to get refresh token");
        let expire = {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            Duration::from_secs(token.expires_in_secs) + now
        };
        let drive = OneDrive::new_with_client(client.clone(), access_token, location.clone());

        Ok(Self {
            drive,
            auth,
            refresh_token,
            client_secret,
            expire,
            client,
            location,
        })
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub async fn refresh_if_expired(&mut self) -> Result<(), onedrive_api::Error> {
        if SystemTime::now().duration_since(UNIX_EPOCH).unwrap() > self.expire {
            let token = self
                .auth
                .login_with_refresh_token(&self.refresh_token, Some(&self.client_secret))
                .await?;
            let access_token = token.access_token;
            let refresh_token = token.refresh_token.expect("Fail to get refresh token");
            let expire = {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                Duration::from_secs(token.expires_in_secs) + now
            };
            let drive =
                OneDrive::new_with_client(self.client.clone(), access_token, self.location.clone());

            self.drive = drive;
            self.expire = expire;
            self.refresh_token = refresh_token;
        }

        Ok(())
    }

    pub async fn list_children(
        &mut self,
        item: ItemLocation<'_>,
    ) -> Result<Vec<DriveItem>, onedrive_api::Error> {
        self.refresh_if_expired().await?;
        self.drive.list_children(item).await
    }

    pub async fn get_item_download_url(
        &mut self,
        item: ItemLocation<'_>,
    ) -> Result<String, onedrive_api::Error> {
        self.refresh_if_expired().await?;
        self.drive.get_item_download_url(item).await
    }
}

pub struct OneDriveProvider {
    drive: Mutex<OneDriveClient>,
    client: Client,
    albums: DashMap<String, (String, usize)>, // album_id => (path (without prefix '/'), size)
}

fn format_audio_path(
    base: &str,
    album_id: &str,
    disc_id: NonZeroU8,
    track_id: NonZeroU8,
) -> String {
    if base.is_empty() {
        format!("/{album_id}/{disc_id}/{track_id}.flac")
    } else {
        format!("/{base}/{album_id}/{disc_id}/{track_id}.flac")
    }
}

fn format_cover_path(base: &str, album_id: &str, disc_id: Option<NonZeroU8>) -> String {
    let path = match disc_id {
        Some(id) => format!("/{album_id}/{id}/cover.jpg"),
        None => format!("/{album_id}/cover.jpg"),
    };
    if base.is_empty() {
        path
    } else {
        format!("/{base}{path}")
    }
}

impl OneDriveProvider {
    pub fn with_drive(drive: OneDriveClient) -> Self {
        let client = drive.client();
        Self {
            drive: Mutex::new(drive),
            client,
            albums: DashMap::new(),
        }
    }
    pub async fn new(drive: OneDriveClient) -> Result<Self, Error> {
        let mut p = Self::with_drive(drive);
        p.reload_albums().await?;
        Ok(p)
    }
    pub async fn reload_albums(&mut self) -> Result<(), Error> {
        let items = self
            .drive
            .lock()
            .await
            .list_children(ItemLocation::root())
            .await?;
        let albums = items
            .into_iter()
            .filter_map(|item| match item.name.clone() {
                Some(name) if name.len() == 36 => Some((
                    name,
                    (
                        item.parent_reference?["path"]
                            .as_str()?
                            .split('/')
                            .skip_while(|c| *c != "root:")
                            .skip(1)
                            .collect(),
                        item.size.map(|s| s as usize).unwrap_or_default(),
                    ),
                )),
                _ => None,
            })
            .collect();
        self.albums = albums;
        Ok(())
    }
    pub async fn file_url(&self, path: &str) -> Result<String, Error> {
        let location = ItemLocation::from_path(path).ok_or(ProviderError::InvalidPath)?;
        let mut guard = self.drive.lock().await;
        Ok(guard.get_item_download_url(dbg!(location)).await?)
    }
    pub async fn audio_url(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
    ) -> Result<(String, usize), Error> {
        let (path, size) = match self.albums.get(album_id) {
            Some(p) => (format_audio_path(&p.0, album_id, disc_id, track_id), p.1),
            None => return Err(ProviderError::FileNotFound.into()),
        };
        Ok((self.file_url(&path).await?, size))
    }
}

#[async_trait::async_trait]
impl AnniProvider for OneDriveProvider {
    async fn albums(&self) -> anni_provider::Result<HashSet<Cow<str>>> {
        Ok(self
            .albums
            .iter()
            .map(|item| Cow::Owned(item.key().to_owned()))
            .collect())
    }

    async fn get_audio(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
        range: Range,
    ) -> anni_provider::Result<AudioResourceReader> {
        let (url, size) = self.audio_url(album_id, disc_id, track_id).await?;
        let req = self.client.get(url);
        let req = match range.to_range_header() {
            Some(h) => req.header(RANGE, h),
            None => req,
        };
        let resp = req.send().await?;
        let range = content_range_to_range(
            resp.headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok()),
        );
        let reader = StreamReader::new(resp.bytes_stream().map(|result| {
            result.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
        }));
        let (duration, reader) = info::read_duration(Box::pin(reader), range).await?;
        Ok(AudioResourceReader {
            info: AudioInfo {
                extension: String::from("flac"),
                size,
                duration,
            },
            range,
            reader,
        })
    }

    /// Returns a cover of corresponding album
    async fn get_cover(
        &self,
        album_id: &str,
        disc_id: Option<NonZeroU8>,
    ) -> anni_provider::Result<ResourceReader> {
        let path = match self.albums.get(album_id) {
            Some(p) => format_cover_path(&p.0, album_id, disc_id),
            None => return Err(ProviderError::FileNotFound),
        };
        let url = self.file_url(&path).await?;
        let resp = self.client.get(url).send().await?;
        let reader = StreamReader::new(resp.bytes_stream().map(|result| {
            result.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
        }));
        Ok(Box::pin(reader))
    }

    /// Reloads the provider for new albums
    async fn reload(&mut self) -> anni_provider::Result<()> {
        self.reload_albums().await?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum Error {
    ProviderError(ProviderError),
    OneDriveError(onedrive_api::Error),
}

impl From<ProviderError> for Error {
    fn from(value: ProviderError) -> Self {
        Self::ProviderError(value)
    }
}

impl From<onedrive_api::Error> for Error {
    fn from(value: onedrive_api::Error) -> Self {
        Self::OneDriveError(value)
    }
}

impl From<Error> for ProviderError {
    fn from(value: Error) -> Self {
        match value {
            Error::ProviderError(e) => e,
            Error::OneDriveError(_) => ProviderError::GeneralError,
        }
    }
}

fn content_range_to_range(content_range: Option<&str>) -> Range {
    match content_range {
        Some(content_range) => {
            // if content range header is invalid, return the full range
            if content_range.len() <= 6 {
                return Range::FULL;
            }

            // else, parse the range
            // Content-Range: bytes 0-1023/10240
            //                      | offset = 6
            let content_range = &content_range[6..];
            let (from, content_range) =
                content_range.split_once('-').unwrap_or((content_range, ""));
            let (to, total) = content_range.split_once('/').unwrap_or((content_range, ""));

            Range {
                start: from.parse().unwrap_or(0),
                end: to.parse().ok(),
                total: total.parse().ok(),
            }
        }
        None => Range::FULL,
    }
}
