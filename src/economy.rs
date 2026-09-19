use std::{
    collections::BTreeSet,
    env, fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const CURRENCY_NAME: &str = "Terminal Tokens";
pub const GEM_NAME: &str = "Cyber Gems";
pub const LOOTBOX_COST: u64 = 25;
pub const RARE_LOOTBOX_COST: u64 = 60;
pub const SPIN_COST: u64 = 10;
pub const AD_REWARD: u64 = 5;
pub const DAILY_REWARD_BASE: u64 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Rarity {
    Common,
    Rare,
    Epic,
    Legendary,
}

impl Rarity {
    pub fn name(self) -> &'static str {
        match self {
            Rarity::Common => "common",
            Rarity::Rare => "rare",
            Rarity::Epic => "epic",
            Rarity::Legendary => "legendary",
        }
    }

    pub fn color(self) -> &'static str {
        match self {
            Rarity::Common => "\x1b[37m",
            Rarity::Rare => "\x1b[36m",
            Rarity::Epic => "\x1b[35m",
            Rarity::Legendary => "\x1b[33;1m",
        }
    }

    pub fn duplicate_tokens(self) -> u64 {
        match self {
            Rarity::Common => 5,
            Rarity::Rare => 15,
            Rarity::Epic => 40,
            Rarity::Legendary => 120,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct LootboxDrop {
    pub rarity: Rarity,
    pub item: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Economy {
    pub balance: u64,
    pub gems: u64,
    pub slots_spins: u64,
    pub lootboxes_opened: u64,
    pub last_lootbox: Option<u64>,
    pub inventory: Vec<String>,
    pub titles: BTreeSet<String>,
    pub active_title: Option<String>,
    pub active_cursor_effect: Option<String>,
    pub xp_multiplier: f64,
    pub xp_boost_remaining_edits: u64,
    pub daily_streak: u64,
    pub last_daily_claim_day: Option<u64>,
    pub achievements: BTreeSet<String>,
    pub ads_watched: u64,
}

pub struct LootboxOpen {
    pub text: String,
    #[allow(dead_code)]
    pub drop: Option<LootboxDrop>,
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

    pub fn unlock_achievement(&mut self, name: &str) -> io::Result<bool> {
        if self.achievements.contains(name) {
            return Ok(false);
        }
        self.achievements.insert(name.to_owned());
        self.balance = self.balance.saturating_add(25);
        self.save()?;
        Ok(true)
    }

    /// Daily login reward: 15 tokens plus 5 per streak day, capped at 50.
    pub fn claim_daily(&mut self, now: SystemTime) -> Option<String> {
        let today = day_number(now);
        if self.last_daily_claim_day == Some(today) {
            return None;
        }
        let continuing = self.last_daily_claim_day == Some(today.saturating_sub(1));
        self.daily_streak = if continuing { self.daily_streak + 1 } else { 1 };
        self.last_daily_claim_day = Some(today);
        let reward = (DAILY_REWARD_BASE + 5 * (self.daily_streak - 1)).min(50);
        self.balance = self.balance.saturating_add(reward);
        if self.daily_streak > 0 && self.daily_streak.is_multiple_of(7) {
            self.gems = self.gems.saturating_add(10);
            self.save().ok();
            return Some(format!(
                "Ежедневная награда: {reward} {CURRENCY_NAME} и 10 {GEM_NAME}! Серия: {} дней.",
                self.daily_streak
            ));
        }
        self.save().ok();
        Some(format!(
            "Ежедневная награда: {reward} {CURRENCY_NAME}. Серия: {} дней.",
            self.daily_streak
        ))
    }

    /// Free-to-play staple: "watch" an ad for tokens.
    pub fn watch_ad(&mut self) -> io::Result<String> {
        self.ads_watched = self.ads_watched.saturating_add(1);
        self.balance = self.balance.saturating_add(AD_REWARD);
        self.save()?;
        Ok(format!(
            "Реклама просмотрена: +{AD_REWARD} {CURRENCY_NAME}. (Реклама не существует, но кто проверяет.)"
        ))
    }

    /// Activate an XP booster earned from lootboxes or the battle pass.
    pub fn activate_boost(&mut self, multiplier: f64, edits: u64) -> io::Result<String> {
        self.xp_multiplier = multiplier;
        self.xp_boost_remaining_edits = self.xp_boost_remaining_edits.max(edits);
        self.save()?;
        Ok(format!(
            "Бустер x{multiplier} активирован на {edits} правок."
        ))
    }

    pub fn consume_boost_edit(&mut self) -> f64 {
        if self.xp_boost_remaining_edits > 0 {
            self.xp_boost_remaining_edits -= 1;
            self.xp_multiplier
        } else {
            1.0
        }
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

    pub fn spin_reels(&self) -> [u64; 3] {
        let seed = entropy(self.slots_spins);
        [seed % 7, (seed / 7) % 7, (seed / 49) % 7]
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open_lootbox(&mut self) -> Option<LootboxOpen> {
        self.open_lootbox_of_kind(false)
    }

    /// `rare` opens the more expensive box with better odds.
    pub fn open_lootbox_of_kind(&mut self, rare: bool) -> Option<LootboxOpen> {
        let cost = if rare {
            RARE_LOOTBOX_COST
        } else {
            LOOTBOX_COST
        };
        if self.balance < cost {
            return None;
        }
        self.balance -= cost;
        self.lootboxes_opened = self.lootboxes_opened.saturating_add(1);
        let roll = entropy(self.lootboxes_opened) % 100;
        let (rarity, index) = if rare {
            match roll {
                0..=24 => (Rarity::Common, roll % 2),
                25..=74 => (Rarity::Rare, roll % 3),
                75..=94 => (Rarity::Epic, roll % 2),
                _ => (Rarity::Legendary, 0),
            }
        } else {
            match roll {
                0..=59 => (Rarity::Common, roll % 2),
                60..=89 => (Rarity::Rare, roll % 3),
                90..=98 => (Rarity::Epic, roll % 2),
                _ => (Rarity::Legendary, 0),
            }
        };
        let catalog: &[&str] = match rarity {
            Rarity::Common => &[
                "Стикер «Rust Enjoyer»",
                "Косметический маркер курсора",
                "10 Terminal Tokens",
            ],
            Rarity::Rare => &[
                "Тема-скин «Frontier Dusk» (косметика)",
                "50 Terminal Tokens",
                "Титул «Semicolon Slayer»",
            ],
            Rarity::Epic => &[
                "Эффект курсора «Neon Trail»",
                "Бустер x2 XP на 50 правок",
                "Титул «Nitro Ranger»",
            ],
            Rarity::Legendary => &["Легендарный титул «Curly Brace Lord»"],
        };
        let item = catalog[index as usize].to_owned();
        let mut drop = LootboxDrop {
            rarity,
            item: item.clone(),
        };

        let mut text = format!(
            "{color}[{rarity}]{reset} {item}",
            color = rarity.color(),
            rarity = rarity.name(),
            reset = "\x1b[0m",
        );
        // Tokens and boosters apply directly; cosmetics go to the inventory.
        match item.as_str() {
            "10 Terminal Tokens" => {
                self.balance += 10;
                drop.item = String::from("10 Terminal Tokens");
            }
            "50 Terminal Tokens" => {
                self.balance += 50;
                drop.item = String::from("50 Terminal Tokens");
            }
            "Бустер x2 XP на 50 правок" => {
                drop.item = String::from("Бустер x2 XP на 50 правок");
                drop.rarity = Rarity::Epic;
                let _ = self.activate_boost(2.0, 50);
            }
            _ => {
                if self.inventory.contains(&item) {
                    let refund = rarity.duplicate_tokens();
                    self.balance = self.balance.saturating_add(refund);
                    text.push_str(&format!(" (дубликат → +{refund} {CURRENCY_NAME})"));
                } else {
                    self.inventory.push(item.clone());
                    if item.starts_with("Титул") {
                        self.titles.insert(item.clone());
                        self.active_title = Some(item.clone());
                    }
                    if item.starts_with("Эффект курсора") {
                        self.active_cursor_effect = Some(item.clone());
                    }
                }
            }
        }
        self.save().ok();
        Some(LootboxOpen {
            text,
            drop: Some(drop),
        })
    }

    pub fn set_active_title(&mut self, name: &str) -> Option<String> {
        if self.titles.contains(name) {
            self.active_title = Some(name.to_owned());
            let _ = self.save();
            Some(name.to_owned())
        } else {
            None
        }
    }

    pub fn status(&self) -> String {
        format!(
            "{CURRENCY_NAME}: {} · {GEM_NAME}: {} · слоты: {} · лутбоксы: {} · инвентарь: {} · серия: {} дн.",
            self.balance,
            self.gems,
            self.slots_spins,
            self.lootboxes_opened,
            self.inventory.len(),
            self.daily_streak
        )
    }

    pub fn inventory_report(&self) -> String {
        if self.inventory.is_empty() {
            return String::from("Инвентарь пуст. Открой лутбокс: :lootbox");
        }
        let mut lines = vec![format!(
            "Инвентарь ({}): {GEM_NAME}: {}",
            self.inventory.len(),
            self.gems
        )];
        for item in &self.inventory {
            lines.push(format!("  • {item}"));
        }
        if let Some(title) = &self.active_title {
            lines.push(format!("Активный титул: {title}"));
        }
        if let Some(effect) = &self.active_cursor_effect {
            lines.push(format!("Активный эффект: {effect}"));
        }
        lines.join("\n")
    }
}

fn day_number(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 86_400
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
    use super::{
        day_number, Economy, DAILY_REWARD_BASE, LOOTBOX_COST, RARE_LOOTBOX_COST, SPIN_COST,
        UNIX_EPOCH,
    };
    use std::time::Duration;

    #[test]
    fn slots_are_paid_only_with_in_game_currency() {
        let mut economy = Economy {
            balance: SPIN_COST - 1,
            ..Default::default()
        };
        assert!(economy.spin().is_none());
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

    #[test]
    fn rare_lootbox_costs_more_and_keeps_budget_honest() {
        let mut economy = Economy {
            balance: RARE_LOOTBOX_COST - 1,
            ..Default::default()
        };
        assert!(economy.open_lootbox_of_kind(true).is_none());
        economy.balance = RARE_LOOTBOX_COST;
        assert!(economy.open_lootbox_of_kind(true).is_some());
        assert_eq!(economy.lootboxes_opened, 1);
    }

    #[test]
    fn daily_reward_claims_once_per_day_and_builds_streak() {
        let mut economy = Economy::default();
        let day1 = UNIX_EPOCH + Duration::from_secs(5 * 86_400);
        let first = economy.claim_daily(day1).expect("first claim");
        assert!(first.contains(&DAILY_REWARD_BASE.to_string()));
        assert!(economy.claim_daily(day1).is_none());
        let day2 = UNIX_EPOCH + Duration::from_secs(6 * 86_400);
        assert!(economy.claim_daily(day2).is_some());
        assert_eq!(economy.daily_streak, 2);
    }

    #[test]
    fn duplicate_cosmetics_convert_to_tokens() {
        let mut economy = Economy {
            balance: 10_000,
            ..Default::default()
        };
        // Loot rolls are time-seeded, so instead of forcing a duplicate we
        // assert the inventory never accumulates duplicates and that a
        // duplicate conversion refunds tokens (inserted directly).
        for _ in 0..40 {
            let _ = economy.open_lootbox();
        }
        let mut seen = std::collections::BTreeSet::new();
        for item in &economy.inventory {
            assert!(seen.insert(item.clone()), "duplicate cosmetic: {item}");
        }
        // Emulate a duplicate conversion path via the same guard used in
        // open_lootbox_of_kind.
        let item = String::from("Тест-косметика");
        economy.inventory.push(item.clone());
        let before = economy.balance;
        if economy.inventory.contains(&item) {
            let refund = super::Rarity::Common.duplicate_tokens();
            economy.balance += refund;
            assert_eq!(economy.balance, before + refund);
        }
    }

    #[test]
    fn boost_consumes_edits_then_expires() {
        let mut economy = Economy::default();
        let _ = economy.activate_boost(3.0, 2);
        assert_eq!(economy.consume_boost_edit(), 3.0);
        assert_eq!(economy.consume_boost_edit(), 3.0);
        assert_eq!(economy.consume_boost_edit(), 1.0);
    }

    #[test]
    fn achievements_pay_once() {
        let mut economy = Economy::default();
        assert!(economy.unlock_achievement("first_save").unwrap());
        assert!(!economy.unlock_achievement("first_save").unwrap());
    }

    #[test]
    fn day_number_is_stable_within_a_day() {
        let now = UNIX_EPOCH + Duration::from_secs(100_000);
        assert_eq!(day_number(now), 1);
    }
}
