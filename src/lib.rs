pub mod info;
pub mod mp3;

pub use anni_provider::{AnniProvider, ProviderError};
pub use onedrive_api;

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fmt::Display,
    num::NonZeroU8,
    sync::{atomic::AtomicU64, Arc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anni_provider::{AudioInfo, AudioResourceReader, Range, ResourceReader};
use onedrive_api::{
    option::ObjectOption,
    resource::{DriveItem, DriveItemField},
    Auth, DriveLocation, ItemLocation, OneDrive, Permission,
};
use reqwest::{
    header::{CONTENT_RANGE, RANGE},
    redirect::Policy,
    Client, ClientBuilder,
};
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub refresh_token: String,
    pub client_secret: String,
    pub location: DriveLocation,
}

impl ClientInfo {
    pub fn new(refresh_token: String, client_secret: String, location: DriveLocation) -> Self {
        Self {
            refresh_token,
            client_secret,
            location,
        }
    }
}

/// A wrapper on [`OneDrive`](onedive_api::OneDrive) that automatically refreshes auth.
#[derive(Debug)]
pub struct OneDriveClient {
    drive: RwLock<OneDrive>,
    auth: Auth,
    expire: AtomicU64,
    client_info: RwLock<ClientInfo>,
    client: Client,
}

impl OneDriveClient {
    /// Creates a new client.
    /// To get the required client_id, refresh_token and client_secret, you can take [rclone's doc](https://rclone.org/onedrive/#getting-your-own-client-id-and-key) as a reference.
    pub async fn new(client_id: String, info: ClientInfo) -> Result<Self, Error> {
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
            .login_with_refresh_token(&info.refresh_token, Some(&info.client_secret))
            .await?;
        let access_token = token.access_token;
        let refresh_token = token.refresh_token.expect("Fail to get refresh token");
        let expire = (Duration::from_secs(token.expires_in_secs) + now()).as_secs();
        let drive = OneDrive::new_with_client(client.clone(), access_token, info.location.clone());

        Ok(Self {
            drive: RwLock::new(drive),
            auth,
            client_info: RwLock::new(ClientInfo {
                refresh_token,
                ..info
            }),
            expire: AtomicU64::new(expire),
            client,
        })
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub fn expire(&self) -> u64 {
        self.expire.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn set_expire(&self, val: u64) {
        self.expire.store(val, std::sync::atomic::Ordering::Release)
    }

    pub fn is_expired(&self) -> bool {
        now().as_secs() > self.expire()
    }

    pub async fn client_info(&self) -> tokio::sync::RwLockReadGuard<'_, ClientInfo> {
        self.client_info.read().await
    }

    pub async fn refresh(&self) -> Result<(), onedrive_api::Error> {
        let mut info = self.client_info.write().await;
        if !self.is_expired() {
            return Ok(());
        }
        let token = self
            .auth
            .login_with_refresh_token(&info.refresh_token, Some(&info.client_secret))
            .await?;
        let access_token = token.access_token;
        let refresh_token = token.refresh_token.expect("Fail to get refresh token");
        let expire = Duration::from_secs(token.expires_in_secs) + now();

        let drive =
            OneDrive::new_with_client(self.client.clone(), access_token, info.location.clone());
        *self.drive.write().await = drive;

        self.set_expire(expire.as_secs());
        info.refresh_token = refresh_token;

        Ok(())
    }

    pub async fn refresh_if_expired(&self) -> Result<(), onedrive_api::Error> {
        if self.is_expired() {
            log::debug!("auth expired, refreshing");
            self.refresh().await?;
        }

        Ok(())
    }

    pub async fn list_children(
        &self,
        item: ItemLocation<'_>,
    ) -> Result<Vec<DriveItem>, onedrive_api::Error> {
        #[cfg(feature = "auto-refresh")]
        self.refresh_if_expired().await?;
        self.drive.read().await.list_children(item).await
    }

    pub async fn get_item_download_url(
        &self,
        item: ItemLocation<'_>,
    ) -> Result<String, onedrive_api::Error> {
        #[cfg(feature = "auto-refresh")]
        self.refresh_if_expired().await?;
        self.drive.read().await.get_item_download_url(item).await
    }

    pub async fn get_item(
        &self,
        item: ItemLocation<'_>,
        option: ObjectOption<DriveItemField>,
    ) -> Result<DriveItem, onedrive_api::Error> {
        #[cfg(feature = "auto-refresh")]
        self.refresh_if_expired().await?;
        self.drive
            .read()
            .await
            .get_item_with_option(item, option)
            .await
            .transpose()
            .unwrap()
    }
}

#[derive(Debug)]
pub struct OneDriveProvider {
    pub drive: Arc<OneDriveClient>,
    pub layers: usize,
    pub path: String,
    pub(crate) extension: String,
    client: Client,
    albums: HashMap<String, String>, // album_id => path without prefix '/'
}

impl OneDriveProvider {
    /// `path` should be the root of an [Anni strict directory](https://book.anni.rs/01.audio-convention/09.directory-strict.html).
    ///
    /// **Currently only `layers == 0` is supported.**
    ///
    /// Panics if layers > 4. See [Anni audio convention](https://book.anni.rs/01.audio-convention/09.directory-strict.html)
    pub fn with_drive(drive: Arc<OneDriveClient>, path: String, layers: usize) -> Self {
        assert!(layers <= 4);
        let client = drive.client();
        Self {
            drive,
            layers,
            path,
            extension: String::from("flac"),
            client,
            albums: HashMap::new(),
        }
    }

    /// `path` should be the root of an [Anni strict directory](https://book.anni.rs/01.audio-convention/09.directory-strict.html).
    ///
    /// Panics if layers > 4. See [Anni audio convention](https://book.anni.rs/01.audio-convention/09.directory-strict.html)
    pub async fn new(
        drive: Arc<OneDriveClient>,
        path: String,
        layers: usize,
    ) -> Result<Self, Error> {
        let mut p = Self::with_drive(drive, path, layers);
        p.reload_albums().await?;
        Ok(p)
    }

    pub async fn reload_albums(&mut self) -> Result<(), Error> {
        let items = self
            .drive
            .list_children(ItemLocation::from_path(&self.path).ok_or(ProviderError::InvalidPath)?)
            .await?;
        let albums = items.into_iter().filter_map(|item| {
            Some((
                item.name.clone()?,
                item.parent_reference?["path"]
                    .as_str()?
                    .split('/')
                    .skip_while(|c| *c != "root:")
                    .skip(1)
                    .collect(), // get parent path
            ))
        });

        self.albums.clear();
        self.albums.extend(albums);
        Ok(())
    }

    /// Returns an onedrive download url of requested path and its size
    pub async fn file_url(&self, path: &str) -> Result<(String, usize), Error> {
        let location = ItemLocation::from_path(path).ok_or(ProviderError::InvalidPath)?;
        let item = self.drive.get_item(location, Default::default()).await?;
        let download_url = item.download_url.ok_or(ProviderError::FileNotFound)?;
        let size = item.size.unwrap_or_default();
        Ok((download_url, size as usize))
    }

    /// Returns an onedrive download url of requested audio and its size.
    pub async fn audio_url(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
    ) -> Result<(String, usize), Error> {
        let path = match self.albums.get(album_id) {
            Some(p) => format_audio_path(p, album_id, disc_id, track_id, &self.extension),
            None => return Err(ProviderError::FileNotFound.into()),
        };
        self.file_url(&path).await
    }

    /// Returns an onedrive download url of requested cover.
    pub async fn cover_url(
        &self,
        album_id: &str,
        disc_id: Option<NonZeroU8>,
    ) -> Result<String, Error> {
        let path = match self.albums.get(album_id) {
            Some(p) => format_cover_path(p, album_id, disc_id),
            None => return Err(ProviderError::FileNotFound.into()),
        };
        self.file_url(&path).await.map(|(url, _)| url)
    }
}

#[async_trait::async_trait]
impl AnniProvider for OneDriveProvider {
    async fn albums(&self) -> anni_provider::Result<HashSet<Cow<str>>> {
        log::debug!("getting albums");
        Ok(self.albums.keys().map(Into::into).collect())
    }

    async fn get_audio(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
        range: Range,
    ) -> anni_provider::Result<AudioResourceReader> {
        log::debug!(
            "getting audio {album_id}/{disc_id}/{track_id} {}-{:?}",
            range.start,
            range.end
        );
        let (url, size) = self.audio_url(album_id, disc_id, track_id).await?;
        log::debug!("audio {album_id}/{disc_id}/{track_id} has a size of {size}");
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
        let reader = StreamReader::new(resp.bytes_stream().map(to_io_error));
        let (duration, reader) = info::read_duration(Box::pin(reader), range).await?;
        Ok(AudioResourceReader {
            info: AudioInfo {
                extension: self.extension.clone(),
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
        let url = self.cover_url(album_id, disc_id).await?;
        let resp = self.client.get(url).send().await?;
        let reader = StreamReader::new(resp.bytes_stream().map(to_io_error));
        Ok(Box::pin(reader))
    }

    /// Reloads the provider for new albums
    async fn reload(&mut self) -> anni_provider::Result<()> {
        log::debug!("reloading albums");
        self.reload_albums().await.map_err(Into::into)
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

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProviderError(e) => write!(f, "{e}"),
            Self::OneDriveError(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

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

fn to_io_error<T, E: Into<Box<dyn std::error::Error + Send + Sync>>>(
    r: Result<T, E>,
) -> Result<T, std::io::Error> {
    r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

fn format_audio_path(
    base: &str,
    album_id: &str,
    disc_id: NonZeroU8,
    track_id: NonZeroU8,
    ext: &str,
) -> String {
    if base.is_empty() {
        format!("/{album_id}/{disc_id}/{track_id}.{ext}")
    } else {
        format!("/{base}/{album_id}/{disc_id}/{track_id}.{ext}")
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

fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
}
