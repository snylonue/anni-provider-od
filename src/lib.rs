pub mod info;

use anni_provider::{
    AnniProvider, AudioInfo, AudioResourceReader, ProviderError, Range, ResourceReader,
};
use dashmap::DashMap;
use onedrive_api::{ItemLocation, OneDrive};
use reqwest::header::{CONTENT_RANGE, RANGE};
use std::{borrow::Cow, collections::HashSet, num::NonZeroU8};
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

pub struct OneDriveProvider {
    drive: OneDrive,
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
    pub fn with_drive(drive: OneDrive) -> Self {
        Self {
            drive,
            albums: DashMap::new(),
        }
    }
    pub async fn new(drive: OneDrive) -> Result<Self, Error> {
        let mut p = Self::with_drive(drive);
        p.reload_albums().await?;
        Ok(p)
    }
    pub async fn reload_albums(&mut self) -> Result<(), Error> {
        let items = self.drive.list_children(ItemLocation::root()).await?;
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
        self.albums = dbg!(albums);
        Ok(())
    }
    pub async fn file_url(&self, path: &str) -> Result<String, Error> {
        let location = ItemLocation::from_path(path).ok_or(ProviderError::InvalidPath)?;
        Ok(self.drive.get_item_download_url(location).await?)
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
        let req = self.drive.client().get(url);
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
        let (duration, reader) = info::read_duration(Box::pin(reader), range)
            .await
            .map_err(ProviderError::FlacError)?;
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
        let resp = self.drive.client().get(url).send().await?;
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
