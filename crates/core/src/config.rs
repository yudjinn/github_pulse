//! The owner's `config.toml`, shared by the CLI (which reads the file) and the
//! Worker (which receives the non-secret parts as `[vars]` and the rest as secrets).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub worker: WorkerConfig,
    pub profile: ProfileConfig,
    pub location: LocationConfig,
    #[serde(default)]
    pub secrets: Secrets,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Worker script name on Cloudflare.
    pub name: String,
    /// Custom domain the Worker is served from. Its zone must be on the account.
    pub domain: Option<String>,
    /// KV namespace id for the `PULSE` binding.
    pub kv_namespace_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub github_user: String,
    /// `YYYY-MM`, the start of the `uptime: career up …` line.
    pub career_start: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationConfig {
    /// Short label shown on the weather line, e.g. `helsinki`.
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    /// IANA zone, e.g. `Europe/Helsinki`.
    pub timezone: String,
}

/// Values that are only ever sent via `wrangler secret`, never written to `wrangler.toml`.
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Secrets {
    #[serde(default)]
    pub spotify_client_id: String,
    #[serde(default)]
    pub spotify_client_secret: String,
    #[serde(default)]
    pub spotify_refresh_token: String,
    #[serde(default)]
    pub github_token: String,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secrets { .. }")
    }
}

impl Secrets {
    /// `(binding name, value)` pairs, skipping empty values.
    pub fn bindings(&self) -> Vec<(&'static str, &str)> {
        [
            (secret::SPOTIFY_CLIENT_ID, self.spotify_client_id.as_str()),
            (secret::SPOTIFY_CLIENT_SECRET, self.spotify_client_secret.as_str()),
            (secret::SPOTIFY_REFRESH_TOKEN, self.spotify_refresh_token.as_str()),
            (secret::GITHUB_TOKEN, self.github_token.as_str()),
        ]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .collect()
    }
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// `(binding name, value)` pairs exposed to the Worker as plain `[vars]`.
    pub fn vars(&self) -> Vec<(&'static str, String)> {
        vec![
            (var::GITHUB_USER, self.profile.github_user.clone()),
            (var::CAREER_START, self.profile.career_start.clone()),
            (var::LOCATION_NAME, self.location.name.clone()),
            (var::LATITUDE, self.location.latitude.to_string()),
            (var::LONGITUDE, self.location.longitude.to_string()),
            (var::TIMEZONE, self.location.timezone.clone()),
        ]
    }
}

/// Binding names for `[vars]`.
pub mod var {
    pub const GITHUB_USER: &str = "GITHUB_USER";
    pub const CAREER_START: &str = "CAREER_START";
    pub const LOCATION_NAME: &str = "LOCATION_NAME";
    pub const LATITUDE: &str = "LATITUDE";
    pub const LONGITUDE: &str = "LONGITUDE";
    pub const TIMEZONE: &str = "TIMEZONE";
}

/// Binding names for secrets.
pub mod secret {
    pub const SPOTIFY_CLIENT_ID: &str = "SPOTIFY_CLIENT_ID";
    pub const SPOTIFY_CLIENT_SECRET: &str = "SPOTIFY_CLIENT_SECRET";
    pub const SPOTIFY_REFRESH_TOKEN: &str = "SPOTIFY_REFRESH_TOKEN";
    pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
}

/// KV namespace binding name.
pub const KV_BINDING: &str = "PULSE";

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../config.example.toml");

    #[test]
    fn example_config_parses() {
        let cfg = Config::from_toml(EXAMPLE).unwrap();
        assert_eq!(cfg.location.timezone, "Europe/Helsinki");
        assert!(cfg.secrets.bindings().is_empty());
    }

    #[test]
    fn secrets_are_not_debug_printed() {
        let s = Secrets {
            github_token: "ghp_secret".into(),
            ..Default::default()
        };
        assert!(!format!("{s:?}").contains("ghp_secret"));
    }
}
