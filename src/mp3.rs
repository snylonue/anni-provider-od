use std::{borrow::Cow, collections::HashSet, num::NonZeroU8, sync::Arc};

use anni_provider::{
    AnniProvider, AudioInfo, AudioResourceReader, ProviderError, Range, ResourceReader,
};
use onedrive_api::{option::ObjectOption, resource::DriveItemField, ItemLocation};
use reqwest::header::{CONTENT_RANGE, RANGE};
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

use crate::{
    content_range_to_range, format_audio_path, to_io_error, Error, OneDriveClient, OneDriveProvider,
};

pub struct Mp3OnedriveProvider {
    provider: OneDriveProvider,
}

impl Mp3OnedriveProvider {
    /// `path` should be the root of an [Anni strict directory](https://book.anni.rs/01.audio-convention/09.directory-strict.html).
    ///
    /// Panics if layers > 4. See [Anni audio convention](https://book.anni.rs/01.audio-convention/09.directory-strict.html)
    pub async fn new(
        drive: Arc<OneDriveClient>,
        path: String,
        layers: usize,
    ) -> Result<Self, Error> {
        let mut provider = OneDriveProvider::with_drive(drive, path, layers);
        provider.reload_albums().await?;
        provider.extension = String::from("mp3");
        Ok(Self { provider })
    }
}

#[async_trait::async_trait]
impl AnniProvider for Mp3OnedriveProvider {
    async fn albums(&self) -> anni_provider::Result<HashSet<Cow<str>>> {
        self.provider.albums().await
    }

    /// Returns whether given album exists
    async fn has_album(&self, album_id: &str) -> bool {
        self.provider.has_album(album_id).await
    }

    /// Get audio info describing basic information of the audio file.
    async fn get_audio_info(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
    ) -> anni_provider::Result<AudioInfo> {
        let path = match self.provider.albums.get(album_id) {
            Some(p) => format_audio_path(p, album_id, disc_id, track_id, &self.provider.extension),
            None => return Err(ProviderError::FileNotFound.into()),
        };
        let location = ItemLocation::from_path(&path).ok_or(ProviderError::InvalidPath)?;

        let info = self
            .provider
            .drive
            .get_item(
                location,
                ObjectOption::new().select(&[DriveItemField::audio]),
            )
            .await
            .map_err(Error::from)?;

        let duration = info
            .audio
            .unwrap()
            .get("duration")
            .unwrap()
            .as_u64()
            .unwrap();
        let size = info.size.unwrap() as usize;

        Ok(AudioInfo {
            extension: self.provider.extension.clone(),
            size,
            duration,
        })
    }

    /// Returns a reader implements AsyncRead for content reading
    async fn get_audio(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
        range: Range,
    ) -> anni_provider::Result<AudioResourceReader> {
        let path = match self.provider.albums.get(album_id) {
            Some(p) => format_audio_path(p, album_id, disc_id, track_id, &self.provider.extension),
            None => return Err(ProviderError::FileNotFound.into()),
        };
        let location = ItemLocation::from_path(&path).ok_or(ProviderError::InvalidPath)?;

        let item = self
            .provider
            .drive
            .get_item(
                location,
                ObjectOption::new().select(&[DriveItemField::audio]),
            )
            .await
            .map_err(Error::from)?;

        let duration = item
            .audio
            .unwrap()
            .get("duration")
            .unwrap()
            .as_u64()
            .unwrap();
        let size = item.size.unwrap() as usize;

        let info = AudioInfo {
            extension: self.provider.extension.clone(),
            size,
            duration,
        };

        let req = self
            .provider
            .client
            .get(item.download_url.ok_or(ProviderError::FileNotFound)?);
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

        Ok(AudioResourceReader {
            info,
            range,
            reader: Box::pin(reader),
        })
    }

    /// Returns a cover of corresponding album
    async fn get_cover(
        &self,
        album_id: &str,
        disc_id: Option<NonZeroU8>,
    ) -> anni_provider::Result<ResourceReader> {
        self.provider.get_cover(album_id, disc_id).await
    }

    /// Reloads the provider for new albums
    async fn reload(&mut self) -> anni_provider::Result<()> {
        self.provider.reload().await
    }
}
