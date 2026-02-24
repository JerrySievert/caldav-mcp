use std::env;

/// Application configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    pub caldav_port: u16,
    pub mcp_port: u16,
    pub database_url: String,
}

impl Config {
    /// Load configuration from environment variables with sensible defaults.
    pub fn from_env() -> Result<Self, env::VarError> {
        Ok(Self {
            caldav_port: env::var("CALDAV_PORT")
                .unwrap_or_else(|_| "5232".to_string())
                .parse()
                .expect("CALDAV_PORT must be a valid port number"),
            mcp_port: env::var("MCP_PORT")
                .unwrap_or_else(|_| "5233".to_string())
                .parse()
                .expect("MCP_PORT must be a valid port number"),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:data/caldav.db?mode=rwc".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_fields_exist() {
        // Verify Config struct has the expected fields and from_env doesn't panic
        // with whatever env is currently set (avoids env var race conditions).
        let config = Config::from_env().unwrap();
        // Ports must be non-zero
        assert!(config.caldav_port > 0);
        assert!(config.mcp_port > 0);
        assert!(!config.database_url.is_empty());
    }
}
