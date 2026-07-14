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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crunchyroll_rs::media::{SkipEvents, StreamData};
    use crunchyroll_rs::{Episode, Movie, MusicVideo, Season, Series};

    use crate::MediaKind;
    use crate::api::ApiSubtitle;

    use super::*;

    #[derive(Default)]
    struct MockApi {
        next: AtomicUsize,
        invalidated: Mutex<Vec<usize>>,
        fail_stream_data: bool,
        cancel_on_open: Option<CancellationToken>,
    }

    impl CrunchyrollApi for MockApi {
        type Session = usize;

        async fn episode_from_id(&self, _id: &str) -> Result<Episode, Error> {
            unreachable!()
        }
        async fn movie_from_id(&self, _id: &str) -> Result<Movie, Error> {
            unreachable!()
        }
        async fn music_video_from_id(&self, _id: &str) -> Result<MusicVideo, Error> {
            unreachable!()
        }
        async fn series_from_id(&self, _id: &str) -> Result<Series, Error> {
            unreachable!()
        }
        async fn season_from_id(&self, _id: &str) -> Result<Season, Error> {
            unreachable!()
        }
        async fn series_seasons(&self, _series: &Series) -> Result<Vec<Season>, Error> {
            unreachable!()
        }
        async fn season_episodes(&self, _season: &Season) -> Result<Vec<Episode>, Error> {
            unreachable!()
        }
        async fn open_playback(
            &self,
            _content_id: &str,
            _platform: &StreamPlatform,
        ) -> Result<Self::Session, Error> {
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            if let Some(cancellation) = &self.cancel_on_open {
                cancellation.cancel();
            }
            Ok(id)
        }
        fn session_metadata(&self, _session: &Self::Session) -> SessionMetadata {
            SessionMetadata {
                audio_locale: Locale::en_US,
                hardsubs: Vec::new(),
                subtitles: Vec::<ApiSubtitle>::new(),
            }
        }
        async fn stream_data(
            &self,
            _session: &Self::Session,
            _hardsub: Option<Locale>,
        ) -> Result<Option<StreamData>, Error> {
            if self.fail_stream_data {
                Err(Error::Manifest("fixture manifest failure".to_string()))
            } else {
                unreachable!()
            }
        }
        async fn invalidate_playback(&self, session: Self::Session) -> Result<(), Error> {
            self.invalidated
                .lock()
                .expect("invalidation lock")
                .push(session);
            Ok(())
        }
        async fn skip_events(
            &self,
            _content_id: &str,
            _kind: MediaKind,
        ) -> Result<Option<SkipEvents>, Error> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn explicitly_invalidates_on_success() {
        let api = MockApi::default();
        let cancellation = CancellationToken::new();
        let mut guard = SessionGuard::new(&api);
        guard
            .open("V1", &StreamPlatform::default(), &cancellation)
            .await
            .expect("session opens");
        guard.finalize().await.expect("finalization succeeds");
        assert_eq!(*api.invalidated.lock().expect("lock"), vec![0]);
    }

    #[tokio::test]
    async fn explicitly_invalidates_after_error() {
        let api = MockApi {
            fail_stream_data: true,
            ..MockApi::default()
        };
        let cancellation = CancellationToken::new();
        let mut guard = SessionGuard::new(&api);
        let index = guard
            .open("V1", &StreamPlatform::default(), &cancellation)
            .await
            .expect("session opens");
        assert!(guard.stream_data(index, None, &cancellation).await.is_err());
        guard.finalize().await.expect("finalization succeeds");
        assert_eq!(*api.invalidated.lock().expect("lock"), vec![0]);
    }

    #[tokio::test]
    async fn explicitly_invalidates_after_cancellation() {
        let cancellation = CancellationToken::new();
        let api = MockApi {
            cancel_on_open: Some(cancellation.clone()),
            ..MockApi::default()
        };
        let mut guard = SessionGuard::new(&api);
        let index = guard
            .open("V1", &StreamPlatform::default(), &cancellation)
            .await
            .expect("session opens before cancellation is observed");
        assert!(matches!(
            guard.stream_data(index, None, &cancellation).await,
            Err(Error::Cancelled)
        ));
        guard.finalize().await.expect("finalization succeeds");
        assert_eq!(*api.invalidated.lock().expect("lock"), vec![0]);
    }
}
