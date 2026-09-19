use std::{env, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const SEASON_LEVELS: u32 = 30;
pub const XP_PER_LEVEL: u64 = 100;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct BattlePass {
    pub season: String,
    pub xp: u64,
    pub claimed_free_level: u32,
    pub claimed_premium_level: u32,
    pub premium: bool,
    pub quests: Vec<Quest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Quest {
    pub id: String,
    pub description: String,
    pub goal: u64,
    pub progress: u64,
    pub xp_reward: u64,
    pub completed: bool,
}

impl Quest {
    fn new(id: &str, description: &str, goal: u64, xp_reward: u64) -> Self {
        Self {
            id: id.to_owned(),
            description: description.to_owned(),
            goal,
            progress: 0,
            xp_reward,
            completed: false,
        }
    }
}

impl Default for BattlePass {
    fn default() -> Self {
        Self {
            season: String::from("Terminal Frontier"),
            xp: 0,
            claimed_free_level: 0,
            claimed_premium_level: 0,
            premium: false,
            quests: Self::default_quests(),
        }
    }
}

impl BattlePass {
    fn default_quests() -> Vec<Quest> {
        vec![
            Quest::new("edits", "Сделай 100 правок в буфере", 100, 150),
            Quest::new("saves", "Сохрани файл 5 раз", 5, 100),
            Quest::new("git", "Используй :git 3 раза", 3, 80),
            Quest::new(
                "terminal",
                "Выполни 3 команды во встроенном терминале",
                3,
                80,
            ),
            Quest::new("lootbox", "Открой 2 лутбокса", 2, 60),
        ]
    }

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

    /// Quest progress; returns XP granted if the quest just completed.
    pub fn add_quest_progress(&mut self, quest_id: &str, amount: u64) -> io::Result<Option<u64>> {
        for quest in &mut self.quests {
            if quest.id == quest_id && !quest.completed {
                quest.progress = (quest.progress + amount).min(quest.goal);
                if quest.progress >= quest.goal {
                    quest.completed = true;
                    let xp = quest.xp_reward;
                    self.xp = self.xp.saturating_add(xp);
                    self.save()?;
                    return Ok(Some(xp));
                }
            }
        }
        self.save().ok();
        Ok(None)
    }

    pub fn level(&self) -> u32 {
        (self.xp / XP_PER_LEVEL) as u32
    }

    pub fn reward(&self, level: u32, premium: bool) -> Option<&'static str> {
        if level == 0 || level > SEASON_LEVELS {
            return None;
        }
        if premium {
            Some(match level {
                1 => "250 Terminal Tokens",
                2 => "10 Cyber Gems",
                3 => "Эффект курсора «Nitro Pulse»",
                5 => "Скин статуса «Neon Baron»",
                7 => "1000 Terminal Tokens",
                10 => "Эксклюзивный титул «Nitro Archivist»",
                15 => "25 Cyber Gems",
                20 => "Анимированный курсор «Hyperdrive»",
                25 => "Легендарный скин «Zero Day»",
                _ => "Премиум-валюта: Cyber Gems",
            })
        } else {
            Some(match level {
                1 => "50 Terminal Tokens",
                2 => "Эмблема Terminal Frontier",
                3 => "Эффект курсора «Frontier»",
                5 => "Титул «Rust Ranger»",
                8 => "Бустер x2 XP на 25 правок",
                12 => "Титул «Buffer Baron»",
                20 => "Сезонный титул «Terminal Legend»",
                _ => "25 Terminal Tokens",
            })
        }
    }

    /// Claim all available levels on the requested track; returns reward lines.
    pub fn claim(&mut self, premium_track: bool) -> Vec<String> {
        let mut rewards = Vec::new();
        if premium_track && !self.premium {
            return vec![String::from(
                "Премиум-дорожка требует RustVim Nitro: :bp premium",
            )];
        }
        let level = self.level();
        let already = if premium_track {
            self.claimed_premium_level
        } else {
            self.claimed_free_level
        };
        for claimed_level in (already + 1)..=level {
            if let Some(reward) = self.reward(claimed_level, premium_track) {
                rewards.push(format!("Ур. {}: {reward}", claimed_level));
            }
        }
        if premium_track {
            self.claimed_premium_level = level;
        } else {
            self.claimed_free_level = level;
        }
        if rewards.is_empty() {
            rewards.push(String::from("Нет доступных наград: качай Battle Pass."));
        }
        self.save().ok();
        rewards
    }

    pub fn status(&self) -> String {
        let level = self.level();
        let mut lines = vec![format!(
            "Battle Pass «{}» · {} · уровень {} ({}/{}) · всего XP {}",
            self.season,
            if self.premium { "premium" } else { "free" },
            level,
            self.xp % XP_PER_LEVEL,
            XP_PER_LEVEL,
            self.xp
        )];
        lines.push(String::from("  Следующие награды:"));
        let premium = self.premium;
        let next_levels = (level + 1..=level + 3)
            .filter(|next| *next <= SEASON_LEVELS)
            .collect::<Vec<_>>();
        for next in next_levels {
            lines.push(format!(
                "  Ур. {} free: {}{}",
                next,
                self.reward(next, false).unwrap_or("—"),
                if premium {
                    format!(" · premium: {}", self.reward(next, true).unwrap_or("—"))
                } else {
                    String::new()
                }
            ));
        }
        lines.push(String::from("  Квесты сезона:"));
        for quest in &self.quests {
            lines.push(format!(
                "  [{}{}/{}] {} (+{} XP)",
                if quest.completed { "✔ " } else { "" },
                quest.progress,
                quest.goal,
                quest.description,
                quest.xp_reward
            ));
        }
        lines.push(String::from(
            "  Забрать награды: :bp claim (free) / :bp claim premium",
        ));
        lines.join("\n")
    }

    pub fn set_premium(&mut self, nitro: bool) {
        self.premium = nitro;
        let _ = self.save();
    }

    pub fn badge(&self) -> String {
        if self.claimed_free_level >= 12 {
            String::from("Rust Ranger")
        } else if self.claimed_free_level >= 5 {
            String::from("Frontier")
        } else {
            format!("BP{}", self.level())
        }
    }

    pub fn frontier_cursor_unlocked(&self) -> bool {
        self.claimed_free_level >= 3
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
    use super::{BattlePass, Quest, SEASON_LEVELS, XP_PER_LEVEL};

    #[test]
    fn levels_and_rewards_progress_locally() {
        let mut pass = BattlePass {
            xp: 100,
            ..Default::default()
        };
        assert_eq!(pass.level(), 1);
        let rewards = pass.claim(false);
        assert_eq!(rewards.len(), 1);
        assert!(rewards[0].contains("50 Terminal Tokens"));
        assert!(pass.claim(false)[0].starts_with("Нет доступных"));
    }

    #[test]
    fn premium_track_requires_nitro() {
        let mut pass = BattlePass {
            xp: 300,
            ..Default::default()
        };
        assert!(pass.claim(true)[0].contains("Nitro"));
        pass.set_premium(true);
        assert_eq!(pass.claim(true).len(), 3);
    }

    #[test]
    fn rewards_cover_whole_season() {
        let pass = BattlePass::default();
        for level in 1..=SEASON_LEVELS {
            assert!(pass.reward(level, false).is_some(), "free level {level}");
            assert!(pass.reward(level, true).is_some(), "premium level {level}");
        }
        assert!(pass.reward(0, false).is_none());
        assert!(pass.reward(SEASON_LEVELS + 1, false).is_none());
    }

    #[test]
    fn quests_grant_xp_once() {
        let mut pass = BattlePass::default();
        let first = pass.add_quest_progress("git", 3).expect("progress");
        assert_eq!(first, Some(80));
        assert_eq!(pass.add_quest_progress("git", 1).unwrap(), None);
        assert!(pass.xp >= 80);
    }

    #[test]
    fn xp_per_level_is_constant() {
        let pass = BattlePass {
            xp: XP_PER_LEVEL * 2 + 5,
            ..Default::default()
        };
        assert_eq!(pass.level(), 2);
        assert_eq!(pass.xp % XP_PER_LEVEL, 5);
        let _ = Quest::new("x", "x", 1, 1);
    }
}
