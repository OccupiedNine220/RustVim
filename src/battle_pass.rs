use std::{env, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct BattlePass {
    pub season: String,
    pub xp: u64,
    pub claimed_level: u32,
    pub premium: bool,
}

impl Default for BattlePass {
    fn default() -> Self {
        Self {
            season: String::from("Terminal Frontier"),
            xp: 0,
            claimed_level: 0,
            premium: false,
        }
    }
}

impl BattlePass {
    pub fn load() -> io::Result<Self> {
        match fs::read_to_string(path()) {
            Ok(content) => Ok(toml::from_str(&content).unwrap_or_default()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let target = path();
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            target,
            toml::to_string_pretty(self).expect("battle pass serializes"),
        )
    }

    pub fn add_xp(&mut self, amount: u64) -> io::Result<()> {
        self.xp = self.xp.saturating_add(amount);
        self.save()
    }

    pub fn level(&self) -> u32 {
        (self.xp / 100) as u32
    }

    pub fn claim(&mut self) -> Option<&'static str> {
        let level = self.level();
        if level <= self.claimed_level {
            return None;
        }
        self.claimed_level = level;
        let reward = match (self.premium, level) {
            (true, 1) => "Премиум-награда: набор Terminal Tokens",
            (true, 2) => "Премиум-награда: эксклюзивный Nitro-статус",
            (true, 3) => "Премиум-награда: анимация курсора Nitro",
            (true, _) => "Премиум-награда: легендарный титул Nitro Ranger",
            (false, 1) => "Награда: эмблема Terminal Frontier",
            (false, 2) => "Награда: уникальный battle-pass статус",
            (false, 3) => "Награда: эффект курсора Frontier",
            (false, 4) => "Награда: сезонный титул Rust Ranger",
            (false, _) => "Награда: легендарный сезонный титул",
        };
        self.save().ok();
        Some(reward)
    }

    pub fn status(&self) -> String {
        format!(
            "Battle Pass {} · {} · level {} · {}/100 XP · claimed {}",
            self.season,
            if self.premium { "premium" } else { "free" },
            self.level(),
            self.xp % 100,
            self.claimed_level
        )
    }

    pub fn set_premium(&mut self, nitro: bool) {
        self.premium = nitro;
        let _ = self.save();
    }

    pub fn badge(&self) -> String {
        if self.claimed_level >= 4 {
            String::from("Rust Ranger")
        } else if self.claimed_level >= 2 {
            String::from("Frontier")
        } else {
            format!("BP{}", self.level())
        }
    }

    pub fn frontier_cursor_unlocked(&self) -> bool {
        self.claimed_level >= 3
    }
}

fn path() -> PathBuf {
    if let Some(path) = env::var_os("RUSTVIM_STATE") {
        return PathBuf::from(path).join("battle-pass.toml");
    }
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rustvim/battle-pass.toml")
}

#[cfg(test)]
mod tests {
    use super::BattlePass;

    #[test]
    fn levels_and_rewards_progress_locally() {
        let mut pass = BattlePass {
            xp: 100,
            ..Default::default()
        };
        assert_eq!(pass.level(), 1);
        assert!(pass.claim().is_some());
        assert!(pass.claim().is_none());
    }
}
