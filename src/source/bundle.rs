//! Capture source bytes and a transitive declared module DAG before execution.
use super::{
    names,
    reader::{budget, read_source},
};
use crate::error::{ErrorCode, KoruError, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Bounds for source capture; values are intentionally conservative prototypes.
#[derive(Debug, Clone, Copy)]
pub struct SourceLimits {
    /// Maximum bytes in one command or module.
    pub max_file_bytes: usize,
    /// Maximum bytes in the whole captured bundle.
    pub max_total_bytes: usize,
    /// Maximum number of modules.
    pub max_modules: usize,
    /// Maximum transitive dependency depth, with the entry at depth zero.
    pub max_depth: usize,
}
impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 256 * 1024,
            max_total_bytes: 1024 * 1024,
            max_modules: 32,
            max_depth: 16,
        }
    }
}
impl SourceLimits {
    fn validate(self) -> Result<Self> {
        let ceiling = Self::default();
        if self.max_file_bytes == 0
            || self.max_file_bytes > ceiling.max_file_bytes
            || self.max_total_bytes == 0
            || self.max_total_bytes > ceiling.max_total_bytes
            || self.max_modules > ceiling.max_modules
            || self.max_depth > ceiling.max_depth
        {
            return Err(KoruError::new(
                ErrorCode::Validation,
                "source limits exceed hard ceilings",
            ));
        }
        Ok(self)
    }
}

/// One captured Lua source and its statically declared dependencies.
#[derive(Debug, Clone)]
pub struct CapturedSource {
    pub(super) bytes: Vec<u8>,
    pub(super) dependencies: Vec<String>,
}
impl CapturedSource {
    /// Immutable text from the source snapshot.
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes).expect("validated at capture")
    }
    /// Exact captured bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Declared direct dependencies.
    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }
}
/// A bounded, digest-addressed command source and module closure.
#[derive(Debug, Clone)]
pub struct SourceBundle {
    command: String,
    entry: CapturedSource,
    modules: BTreeMap<String, CapturedSource>,
    digest: [u8; 32],
}
impl SourceBundle {
    /// Capture an installed command and all its declared pure-Lua modules.
    pub fn capture(commands_dir: &Path, command: &str, limits: SourceLimits) -> Result<Self> {
        names::command(command)?;
        let limits = limits.validate()?;
        let entry = read_source(
            &commands_dir.join(format!("{command}.lua")),
            limits.max_file_bytes,
        )?;
        let mut loader = Loader {
            root: commands_dir.join("lib"),
            limits,
            total: entry.bytes.len(),
            modules: BTreeMap::new(),
            visiting: BTreeSet::new(),
        };
        if loader.total > limits.max_total_bytes {
            return Err(budget("source bundle bytes"));
        }
        for dependency in &entry.dependencies {
            loader.capture(dependency, 1)?;
        }
        let mut hasher = Sha256::new();
        hasher.update(b"koru-source-bundle-v1\0");
        frame(&mut hasher, command.as_bytes());
        frame(&mut hasher, &entry.bytes);
        for (name, source) in &loader.modules {
            frame(&mut hasher, name.as_bytes());
            frame(&mut hasher, &source.bytes);
        }
        let digest = hasher.finalize().into();
        Ok(Self {
            command: command.to_owned(),
            entry,
            modules: loader.modules,
            digest,
        })
    }
    /// Installed command name.
    pub fn command(&self) -> &str {
        &self.command
    }
    /// Captured command source.
    pub fn entry(&self) -> &CapturedSource {
        &self.entry
    }
    /// Captured modules, ordered by name.
    pub fn modules(&self) -> &BTreeMap<String, CapturedSource> {
        &self.modules
    }
    /// Captured module by dotted name.
    pub fn module(&self, name: &str) -> Option<&CapturedSource> {
        self.modules.get(name)
    }
    /// SHA-256 digest over framed names and bytes.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    /// Lowercase hexadecimal identity suitable for display.
    pub fn digest_hex(&self) -> String {
        self.digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
struct Loader {
    root: PathBuf,
    limits: SourceLimits,
    total: usize,
    modules: BTreeMap<String, CapturedSource>,
    visiting: BTreeSet<String>,
}
impl Loader {
    fn capture(&mut self, name: &str, depth: usize) -> Result<()> {
        names::module(name)?;
        if depth > self.limits.max_depth {
            return Err(budget("module depth"));
        }
        if self.visiting.contains(name) {
            return Err(KoruError::new(
                ErrorCode::Validation,
                format!("module dependency cycle at {name}"),
            ));
        }
        if self.modules.contains_key(name) {
            return Ok(());
        }
        if self.modules.len() + self.visiting.len() >= self.limits.max_modules {
            return Err(budget("module count"));
        }
        let mut path = self.root.clone();
        for segment in name.split('.') {
            path.push(segment);
        }
        path.set_extension("lua");
        self.visiting.insert(name.to_owned());
        let source = read_source(&path, self.limits.max_file_bytes)?;
        self.total = self
            .total
            .checked_add(source.bytes.len())
            .ok_or_else(|| budget("source bundle bytes"))?;
        if self.total > self.limits.max_total_bytes {
            return Err(budget("source bundle bytes"));
        }
        for dependency in &source.dependencies {
            self.capture(dependency, depth + 1)?;
        }
        self.visiting.remove(name);
        self.modules.insert(name.to_owned(), source);
        Ok(())
    }
}
fn frame(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}
