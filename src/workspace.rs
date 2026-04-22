use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const STATE_DIRS: &[&str] = &["inbox", "working", "proven", "rejected"];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Inbox,
    Working,
    Proven,
    Rejected,
}

impl State {
    pub fn dir_name(self) -> &'static str {
        match self {
            State::Inbox => "inbox",
            State::Working => "working",
            State::Proven => "proven",
            State::Rejected => "rejected",
        }
    }
}

/// A workspace is a directory with the four state subdirs.
/// Its source of truth is the filesystem: transitions are file moves.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Create the workspace layout at `root`, idempotently.
    pub fn init(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .with_context(|| format!("creating workspace root {}", root.display()))?;
        for dir in STATE_DIRS {
            let p = root.join(dir);
            fs::create_dir_all(&p)
                .with_context(|| format!("creating state dir {}", p.display()))?;
        }
        let meta = root.join(".arghda");
        fs::create_dir_all(&meta)
            .with_context(|| format!("creating meta dir {}", meta.display()))?;
        Ok(Self { root })
    }

    /// Open an existing workspace; fails if any state dir is missing.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        for dir in STATE_DIRS {
            let p = root.join(dir);
            if !p.is_dir() {
                anyhow::bail!("workspace missing state dir: {}", p.display());
            }
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_dir(&self, state: State) -> PathBuf {
        self.root.join(state.dir_name())
    }

    /// List files currently in `state`.
    pub fn list(&self, state: State) -> Result<Vec<PathBuf>> {
        let dir = self.state_dir(state);
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("reading state dir {}", dir.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                out.push(entry.path());
            }
        }
        Ok(out)
    }
}
