use std::{collections::HashSet, env, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Represents a DLC pack that can be purchased
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DlcPack {
    pub id: String,
    pub name: String,
    pub description: String,
    pub price_tokens: u64, // Price in Terminal Tokens
    pub price_gems: u64,   // Price in Cyber Gems
    pub features: Vec<String>, // Features unlocked by this DLC
}

/// Manages DLC ownership and purchasing
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DlcManager {
    pub owned_dlcs: HashSet<String>,
}

impl DlcManager {
    pub fn load() -> io::Result<Self> {
        match fs::read_to_string(Self::path()) {
            Ok(content) => Ok(toml::from_str(&content).unwrap_or_default()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let target = Self::path();
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            target,
            toml::to_string_pretty(self).expect("dlc state serializes"),
        )
    }

    pub fn owns(&self, dlc_id: &str) -> bool {
        self.owned_dlcs.contains(dlc_id)
    }

    pub fn purchase(&mut self, dlc_id: &str) -> io::Result<()> {
        self.owned_dlcs.insert(dlc_id.to_string());
        self.save()
    }

    fn path() -> PathBuf {
        if let Some(path) = env::var_os("RUSTVIM_STATE") {
            return PathBuf::from(path).join("dlc.toml");
        }
        let base = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("rustvim/dlc.toml")
    }
}

/// Available DLC packs for purchase
pub fn available_dlc_packs() -> Vec<DlcPack> {
    vec![
        DlcPack {
            id: "retro_theme_pack".to_string(),
            name: "Retro Theme Pack",
            description: "Пакет ретро тем для терминала и редактора",
            price_tokens: 500,
            price_gems: 25,
            features: vec![
                "retro_themes".to_string(),
                "crystal_theme".to_string(),
                "neon_theme".to_string(),
                "vintage_theme".to_string(),
            ],
        },
        DlcPack {
            id: "battle_pass_season_2".to_string(),
            name: "Battle Pass Season 2",
            description: "Второй сезон Battle Pass с новыми наградами и квестами",
            price_tokens: 1000,
            price_gems: 50,
            features: vec![
                "battle_pass_season_2".to_string(),
                "season_2_quests".to_string(),
                "season_2_rewards".to_string(),
            ],
        },
        DlcPack {
            id: "golden_cursor_pack".to_string(),
            name: "Golden Cursor Pack",
            description: "Позолоченный курсор с блестящим шлейфом. Ничего не ускоряет",
            price_tokens: 250,
            price_gems: 10,
            features: vec![
                "golden_cursor".to_string(),
                "sparkle_trail".to_string(),
            ],
        },
        DlcPack {
            id: "pet_rock_pack".to_string(),
            name: "Pet Rock Pack",
            description: "Домашний камень-питомец для статус-бара. Просто сидит там",
            price_tokens: 100,
            price_gems: 5,
            features: vec![
                "pet_rock".to_string(),
                "rock_idle_animation".to_string(),
            ],
        },
    ]
}