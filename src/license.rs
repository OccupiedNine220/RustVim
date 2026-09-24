use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const PRO_TRIAL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct LicenseState {
    pub agreement_accepted: bool,
    pub agreement_version: String,
    pub exam_passed: bool,
    pub exam_score: u8,
    pub exam_passed_at: Option<u64>,
    pub trial_started_at: Option<u64>,
    pub trial_consumed_secs: u64,
    pub plugins_trial_started_at: Option<u64>,
    pub integrity: String,
}

#[derive(Clone, Debug)]
pub struct Access {
    pub pro: bool,
    pub nitro: bool,
    pub pro_trial_remaining: Duration,
    pub plugins: bool,
    pub plugins_trial_remaining: Duration,
    pub agreement_accepted: bool,
}

pub fn state_path() -> PathBuf {
    if let Some(path) = env::var_os("RUSTVIM_STATE") {
        return PathBuf::from(path).join("license.toml");
    }
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rustvim/license.toml")
}

pub fn load() -> io::Result<LicenseState> {
    let path = state_path();
    let mut state = match fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => LicenseState::default(),
        Err(error) => return Err(error),
    };
    if state.integrity != integrity(&state) {
        state = LicenseState {
            integrity: integrity(&LicenseState::default()),
            ..LicenseState::default()
        };
    }
    Ok(state)
}

pub fn save(state: &mut LicenseState) -> io::Result<()> {
    state.integrity = integrity(state);
    let path = state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        toml::to_string_pretty(state).expect("license state serializes"),
    )
}

pub fn access(state: &LicenseState, nitro_env: bool, pro_env: bool, now: SystemTime) -> Access {
    let now = epoch(now);
    let elapsed = state
        .trial_started_at
        .map(|started| now.saturating_sub(started))
        .unwrap_or(0)
        .max(state.trial_consumed_secs);
    let trial_remaining = PRO_TRIAL.saturating_sub(Duration::from_secs(elapsed));
    let plugins_elapsed = state
        .plugins_trial_started_at
        .map(|started| now.saturating_sub(started))
        .unwrap_or(0);
    let plugin_trial_remaining =
        Duration::from_secs(86_400).saturating_sub(Duration::from_secs(plugins_elapsed));
    Access {
        pro: pro_env || trial_remaining > Duration::ZERO,
        nitro: pro_env || nitro_env || trial_remaining > Duration::ZERO,
        pro_trial_remaining: trial_remaining,
        plugins: pro_env || plugin_trial_remaining > Duration::ZERO,
        plugins_trial_remaining: plugin_trial_remaining,
        agreement_accepted: state.agreement_accepted,
    }
}

pub fn start_trial(state: &mut LicenseState, now: SystemTime) -> io::Result<()> {
    if state.trial_started_at.is_none() {
        state.trial_started_at = Some(epoch(now));
        save(state)?;
    }
    Ok(())
}

pub fn start_plugins_trial(state: &mut LicenseState, now: SystemTime) -> io::Result<()> {
    if state.plugins_trial_started_at.is_none() {
        state.plugins_trial_started_at = Some(epoch(now));
        save(state)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn accept_agreement(state: &mut LicenseState) -> io::Result<()> {
    state.agreement_accepted = true;
    save(state)
}

/// Чистое применение успешной аттестации без записи на диск.
pub fn apply_exam_pass(
    state: &mut LicenseState,
    score: u8,
    version: &str,
    now: SystemTime,
) -> io::Result<()> {
    if score < 24 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "exam score below passing threshold",
        ));
    }
    state.exam_passed = true;
    state.exam_score = score;
    state.exam_passed_at = Some(epoch(now));
    state.agreement_version = version.to_owned();
    state.agreement_accepted = true;
    Ok(())
}

/// Фиксирует успешную аттестацию по соглашению версии 2.0.
///
/// Принимается только проходной балл (>= 24/30). Возвращает ошибку,
/// если балл ниже проходного, чтобы исключить случайное принятие.
pub fn record_exam_pass(
    state: &mut LicenseState,
    score: u8,
    version: &str,
    now: SystemTime,
) -> io::Result<()> {
    apply_exam_pass(state, score, version, now)?;
    save(state)
}

fn epoch(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn integrity(state: &LicenseState) -> String {
    let mut hasher = DefaultHasher::new();
    env::var("USER").unwrap_or_default().hash(&mut hasher);
    env::var("HOSTNAME").unwrap_or_default().hash(&mut hasher);
    state.agreement_accepted.hash(&mut hasher);
    state.agreement_version.hash(&mut hasher);
    state.exam_passed.hash(&mut hasher);
    state.exam_score.hash(&mut hasher);
    state.exam_passed_at.hash(&mut hasher);
    state.trial_started_at.hash(&mut hasher);
    state.trial_consumed_secs.hash(&mut hasher);
    state.plugins_trial_started_at.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::{access, apply_exam_pass, LicenseState};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn trial_expires_after_thirty_minutes() {
        let state = LicenseState {
            trial_started_at: Some(100),
            ..Default::default()
        };
        assert!(access(&state, false, false, UNIX_EPOCH + Duration::from_secs(100)).pro);
        assert!(!access(&state, false, false, UNIX_EPOCH + Duration::from_secs(1901)).pro);
    }

    #[test]
    fn nitro_enables_nitro_features_without_granting_pro() {
        let state = LicenseState {
            trial_started_at: Some(100),
            ..Default::default()
        };
        let access = access(&state, true, false, UNIX_EPOCH + Duration::from_secs(1901));
        assert!(access.nitro);
        assert!(!access.pro);
    }

    #[test]
    fn exam_pass_records_agreement_version() {
        let mut state = LicenseState::default();
        assert!(apply_exam_pass(&mut state, 24, "2.0", UNIX_EPOCH).is_ok());
        assert!(state.agreement_accepted);
        assert!(state.exam_passed);
        assert_eq!(state.exam_score, 24);
        assert_eq!(state.agreement_version, "2.0");
    }

    #[test]
    fn exam_fail_does_not_accept_agreement() {
        let mut state = LicenseState::default();
        assert!(apply_exam_pass(&mut state, 23, "2.0", UNIX_EPOCH).is_err());
        assert!(!state.agreement_accepted);
    }
}
