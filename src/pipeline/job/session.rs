use crunchyroll_rs::Locale;
use crunchyroll_rs::media::{StreamData, StreamPlatform};

use crate::api::{CrunchyrollApi, SessionMetadata};
use crate::{CancellationToken, Error};

pub(crate) struct SessionGuard<'a, A: CrunchyrollApi> {
    api: &'a A,
    sessions: Vec<A::Session>,
}

impl<'a, A: CrunchyrollApi> SessionGuard<'a, A> {
    pub(crate) fn new(api: &'a A) -> Self {
        Self {
            api,
            sessions: Vec::new(),
        }
    }

    pub(crate) async fn open(
        &mut self,
        content_id: &str,
        platform: &StreamPlatform,
        cancellation: &CancellationToken,
    ) -> Result<usize, Error> {
        cancellation.check()?;
        let session = self.api.open_playback(content_id, platform).await?;
        self.sessions.push(session);
        Ok(self.sessions.len() - 1)
    }

    pub(crate) fn metadata(&self, index: usize) -> SessionMetadata {
        self.api.session_metadata(&self.sessions[index])
    }

    pub(crate) async fn stream_data(
        &self,
        index: usize,
        hardsub: Option<Locale>,
        cancellation: &CancellationToken,
    ) -> Result<StreamData, Error> {
        cancellation.check()?;
        self.api
            .stream_data(&self.sessions[index], hardsub)
            .await?
            .ok_or_else(|| Error::Unavailable("requested hardsub is unavailable".to_string()))
    }

    pub(crate) async fn finalize(mut self) -> Result<(), Error> {
        let mut first_error = None;
        while let Some(session) = self.sessions.pop() {
            if let Err(error) = self.api.invalidate_playback(session).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
