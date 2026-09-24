#![allow(dead_code)]

use std::{collections::HashSet, env, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Represents a DLC pack that can be purchased
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DlcPack {
    pub id: String,
    pub name: String,
    pub description: String,
    pub price_tokens: u64,     // Price in Terminal Tokens
    pub price_gems: u64,       // Price in Cyber Gems
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
            name: "Retro Theme Pack".to_string(),
            description: "Пакет ретро тем для терминала и редактора".to_string(),
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
            name: "Battle Pass Season 2".to_string(),
            description: "Второй сезон Battle Pass с новыми наградами и квестами".to_string(),
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
            name: "Golden Cursor Pack".to_string(),
            description: "Позолоченный курсор с блестящим шлейфом. Ничего не ускоряет".to_string(),
            price_tokens: 250,
            price_gems: 10,
            features: vec!["golden_cursor".to_string(), "sparkle_trail".to_string()],
        },
        DlcPack {
            id: "pet_rock_pack".to_string(),
            name: "Pet Rock Pack".to_string(),
            description: "Домашний камень-питомец для статус-бара. Просто сидит там".to_string(),
            price_tokens: 100,
            price_gems: 5,
            features: vec!["pet_rock".to_string(), "rock_idle_animation".to_string()],
        },
        DlcPack {
            id: "lucky_slots_pack".to_string(),
            name: "Lucky Slots Pack".to_string(),
            description: "Пак для любителей слотов: удачные барабаны и бонусный блеск".to_string(),
            price_tokens: 750,
            price_gems: 30,
            features: vec!["lucky_slots".to_string(), "golden_reels".to_string()],
        },
        DlcPack {
            id: "rubber_duck_pack".to_string(),
            name: "Rubber Duck Pack".to_string(),
            description: "Резиновая уточка для дебага. Просто смотрит на код".to_string(),
            price_tokens: 300,
            price_gems: 15,
            features: vec!["rubber_duck".to_string(), "duck_stare".to_string()],
        },
        DlcPack {
            id: "invisible_ink_pack".to_string(),
            name: "Invisible Ink Pack".to_string(),
            description: "Невидимые чернила для секретных комментариев. Невидимы даже вам"
                .to_string(),
            price_tokens: 200,
            price_gems: 8,
            features: vec!["invisible_ink".to_string(), "secret_comments".to_string()],
        },
        DlcPack {
            id: "second_cursor_pack".to_string(),
            name: "Second Cursor Pack".to_string(),
            description: "Второй курсор, который повторяет первый с задержкой".to_string(),
            price_tokens: 400,
            price_gems: 20,
            features: vec!["second_cursor".to_string(), "cursor_echo".to_string()],
        },
        DlcPack {
            id: "floppy_sound_pack".to_string(),
            name: "Floppy Sound Pack".to_string(),
            description: "Звук дискеты при каждом сохранении. Только звук".to_string(),
            price_tokens: 150,
            price_gems: 5,
            features: vec!["floppy_sound".to_string(), "save_click".to_string()],
        },
        DlcPack {
            id: "motivational_quotes_pack".to_string(),
            name: "Motivational Quotes Pack".to_string(),
            description: "Мотивационные цитаты в статус-баре. Не мотивируют".to_string(),
            price_tokens: 350,
            price_gems: 12,
            features: vec!["daily_quotes".to_string(), "stale_motivation".to_string()],
        },
    ]
}
