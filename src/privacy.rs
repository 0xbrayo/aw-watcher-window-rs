use regex::Regex;
use tracing::error;

use crate::window::WindowData;

/// Title filters matching Python aw-watcher-window order:
/// 1. each exclude_titles regex → title = "excluded"
/// 2. exclude_title flag → title = "excluded"
pub struct TitleFilter {
    patterns: Vec<Regex>,
    exclude_all: bool,
}

impl TitleFilter {
    pub fn new(exclude_all: bool, patterns: &[String]) -> Result<Self, String> {
        let mut compiled = Vec::with_capacity(patterns.len());
        for p in patterns {
            match Regex::new(&format!("(?i){}", p)) {
                Ok(re) => compiled.push(re),
                Err(e) => {
                    error!("Invalid regex pattern {:?}: {}", p, e);
                    return Err(format!("Invalid regex pattern {:?}: {}", p, e));
                }
            }
        }
        Ok(Self {
            patterns: compiled,
            exclude_all,
        })
    }

    pub fn apply(&self, mut data: WindowData) -> WindowData {
        for re in &self.patterns {
            if re.is_match(&data.title) {
                data.title = "excluded".to_string();
                break;
            }
        }
        if self.exclude_all {
            data.title = "excluded".to_string();
        }
        data
    }
}
