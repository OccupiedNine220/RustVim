use std::{fs, io, path::PathBuf};

#[derive(Clone, Debug)]
pub struct PluginManager {
    root: PathBuf,
}

impl PluginManager {
    pub fn new() -> Self {
        let root = std::env::var_os("RUSTVIM_STATE")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_STATE_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rustvim/plugins");
        Self { root }
    }

    pub fn list(&self) -> io::Result<Vec<String>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut names = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    pub fn install(&self, name: &str) -> io::Result<()> {
        if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid plugin name"));
        }
        fs::create_dir_all(self.root.join(name))?;
        fs::write(self.root.join(name).join("plugin.toml"), format!("name = {name:?}\nenabled = true\n"))
    }

    pub fn remove(&self, name: &str) -> io::Result<()> {
        fs::remove_dir_all(self.root.join(name))
    }
}
