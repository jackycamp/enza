use std::time::{SystemTime, UNIX_EPOCH};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Note {
    pub id: u64,
    pub target: NoteTarget,
    pub body: String,
    pub created_at_ms: u128,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum NoteTarget {
    File {
        file_path: String,
    },
    Hunk {
        file_path: String,
        hunk_header: String,
    },
    Line {
        file_path: String,
        old_lineno: Option<usize>,
        new_lineno: Option<usize>,
    },
    Range {
        file_path: String,
        start_old_lineno: Option<usize>,
        start_new_lineno: Option<usize>,
        end_old_lineno: Option<usize>,
        end_new_lineno: Option<usize>,
    },
}

impl Note {
    pub fn new(id: u64, target: NoteTarget, body: String) -> Self {
        Self {
            id,
            target,
            body,
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0),
        }
    }
}
