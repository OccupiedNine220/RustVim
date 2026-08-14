use std::{
    env, fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const CURRENCY_NAME: &str = "Terminal Tokens";
pub const LOOTBOX_COST: u64 = 25;
pub const SPIN_COST: u64 = 10;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Economy {
    pub balance: u64,
    pub slots_spins: u64,
    pub lootboxes_opened: u64,
    pub last_lootbox: Option<u64>,
}

impl Economy {
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
            toml::to_string_pretty(self).expect("economy serializes"),
        )
    }

    pub fn earn(&mut self, amount: u64) -> io::Result<()> {
        self.balance = self.balance.saturating_add(amount);
        self.save()
    }

    pub fn spin(&mut self) -> Option<u64> {
        if self.balance < SPIN_COST {
            return None;
        }
        self.balance -= SPIN_COST;
        self.slots_spins = self.slots_spins.saturating_add(1);
        let seed = entropy(self.slots_spins);
        let reels = [seed % 7, (seed / 7) % 7, (seed / 49) % 7];
        let reward = if reels[0] == reels[1] && reels[1] == reels[2] {
            SPIN_COST * 10
        } else if reels[0] == reels[1] || reels[1] == reels[2] || reels[0] == reels[2] {
            SPIN_COST * 2
        } else {
            0
        };
        self.balance = self.balance.saturating_add(reward);
        self.save().ok();
        Some(reward)
    }

    pub fn open_lootbox(&mut self) -> Option<&'static str> {
        if self.balance < LOOTBOX_COST {
            return None;
        }
        self.balance -= LOOTBOX_COST;
        self.lootboxes_opened = self.lootboxes_opened.saturating_add(1);
        let reward = match entropy(self.lootboxes_opened) % 4 {
            0 => {
                self.balance += 10;
                "Лутбокс: 10 Terminal Tokens"
            }
            1 => {
                self.balance += 50;
                "Лутбокс: 50 Terminal Tokens"
            }
            2 => "Лутбокс: косметический маркер курсора",
            _ => "Лутбокс: редкий сезонный титул",
        };
        self.save().ok();
        Some(reward)
    }

    pub fn status(&self) -> String {
        format!(
            "{CURRENCY_NAME}: {} · слоты: {} · лутбоксы: {}",
            self.balance, self.slots_spins, self.lootboxes_opened
        )
    }
}

fn path() -> PathBuf {
    if let Some(path) = env::var_os("RUSTVIM_STATE") {
        return PathBuf::from(path).join("economy.toml");
    }
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rustvim/economy.toml")
}

fn entropy(counter: u64) -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ counter.wrapping_mul(6364136223846793005)
}

#[cfg(test)]
mod tests {
    use super::{Economy, LOOTBOX_COST, SPIN_COST};

    #[test]
    fn slots_are_paid_only_with_in_game_currency() {
        let mut economy = Economy {
            balance: SPIN_COST - 1,
            ..Default::default()
        };
        assert_eq!(economy.spin(), None);
        assert_eq!(economy.balance, SPIN_COST - 1);
    }

    #[test]
    fn lootbox_spends_in_game_currency() {
        let mut economy = Economy {
            balance: LOOTBOX_COST,
            ..Default::default()
        };
        assert!(economy.open_lootbox().is_some());
        assert_eq!(economy.lootboxes_opened, 1);
    }
}
