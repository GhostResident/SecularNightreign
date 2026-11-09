#[derive(Debug, Clone)]
pub enum Event {
    Log(String),
    Stage(String),
    Progress {
        done: u64,
        total: Option<u64>,
    },
    Finished {
        ok: bool,
        message: String,
        close_after: bool,
    },
}
