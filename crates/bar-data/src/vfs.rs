//! Case-insensitive view of a BAR repository checkout, like the engine VFS.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub struct Vfs {
    /// Lower-case relative path -> path on disk, for files.
    files: BTreeMap<String, PathBuf>,
    /// Lower-case relative directory (with trailing `/`) -> entries as
    /// (original-case name, is_dir).
    dirs: BTreeMap<String, Vec<(String, bool)>>,
}

impl Vfs {
    pub fn new(root: &Path) -> Result<Self> {
        let mut vfs = Vfs { files: BTreeMap::new(), dirs: BTreeMap::new() };
        vfs.scan(root, "").with_context(|| format!("cannot read BAR repository at {}", root.display()))?;
        Ok(vfs)
    }

    fn scan(&mut self, dir: &Path, rel: &str) -> Result<()> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type()?.is_dir();
            let child = format!("{rel}{name}");
            if is_dir {
                self.scan(&entry.path(), &format!("{child}/"))?;
            } else {
                self.files.insert(child.to_lowercase(), entry.path());
            }
            entries.push((name, is_dir));
        }
        entries.sort();
        self.dirs.insert(rel.to_lowercase(), entries);
        Ok(())
    }

    fn normalize(path: &str) -> String {
        path.replace('\\', "/").trim_start_matches("./").to_lowercase()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.files.contains_key(&Self::normalize(path))
    }

    pub fn read(&self, path: &str) -> Option<String> {
        let p = self.files.get(&Self::normalize(path))?;
        fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    fn dir_key(dir: &str) -> String {
        let mut d = Self::normalize(dir);
        if !d.is_empty() && !d.ends_with('/') {
            d.push('/');
        }
        d
    }

    fn prefix(dir: &str) -> String {
        let mut d = dir.replace('\\', "/");
        if !d.is_empty() && !d.ends_with('/') {
            d.push('/');
        }
        d
    }

    /// Files in `dir` whose name matches `pattern`, as `dir/name`.
    pub fn dir_list(&self, dir: &str, pattern: &str, recursive: bool) -> Vec<String> {
        let mut out = Vec::new();
        self.dir_list_into(&Self::prefix(dir), pattern, recursive, &mut out);
        out
    }

    fn dir_list_into(&self, prefix: &str, pattern: &str, recursive: bool, out: &mut Vec<String>) {
        let Some(entries) = self.dirs.get(&Self::dir_key(prefix)) else { return };
        for (name, is_dir) in entries {
            if *is_dir {
                if recursive {
                    self.dir_list_into(&format!("{prefix}{name}/"), pattern, true, out);
                }
            } else if glob_match(&pattern.to_lowercase(), &name.to_lowercase()) {
                out.push(format!("{prefix}{name}"));
            }
        }
    }

    /// Sub directories of `dir`, as `dir/name/`.
    pub fn sub_dirs(&self, dir: &str) -> Vec<String> {
        let prefix = Self::prefix(dir);
        let Some(entries) = self.dirs.get(&Self::dir_key(dir)) else { return Vec::new() };
        entries.iter().filter(|(_, d)| *d).map(|(n, _)| format!("{prefix}{n}/")).collect()
    }
}

/// Glob match with `*` and `?`.
fn glob_match(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn globs() {
        assert!(glob_match("*.lua", "armcom.lua"));
        assert!(!glob_match("*.lua", "armcom.luac"));
        assert!(glob_match("*", "x"));
        assert!(glob_match("a?c*", "abcdef"));
    }
}
