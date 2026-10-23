use tokio_util::sync::CancellationToken;
use ytui_services::lyrics::Track;

pub struct View {
    pub id: uuid::Uuid,
    pub media_url: Option<String>,
    pub tracks: Vec<Track>,
    pub selected: usize,
    pub opened: bool,
    pub offset: usize,
    pub follow: bool,
    pub loading: bool,
    pub source: &'static str,
    pub error: Option<String>,
    pub cancel: CancellationToken,
}
impl View {
    pub fn new(media_url: Option<String>, source: &'static str) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            media_url,
            tracks: Vec::new(),
            selected: 0,
            opened: false,
            offset: 0,
            follow: true,
            loading: true,
            source,
            error: None,
            cancel: CancellationToken::new(),
        }
    }
}
impl Drop for View {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
