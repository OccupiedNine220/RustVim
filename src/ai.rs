use std::{env, error::Error, time::Duration};

use async_openai::{config::OpenAIConfig, types::responses::CreateResponseArgs, Client};
use tokio::runtime::Runtime;

const DEFAULT_MODEL: &str = "gpt-5-mini";
const API_BASE_ENV_VAR: &str = "RUSTVIM_AI_BASE_URL";
const API_KEY_ENV_VAR: &str = "RUSTVIM_AI_API_KEY";
const MODEL_ENV_VAR: &str = "RUSTVIM_AI_MODEL";
const MAX_OUTPUT_TOKENS_ENV_VAR: &str = "RUSTVIM_AI_MAX_OUTPUT_TOKENS";
const TIMEOUT_ENV_VAR: &str = "RUSTVIM_AI_TIMEOUT_SECONDS";

pub struct AiClient {
    client: Client<OpenAIConfig>,
    model: String,
    runtime: Runtime,
    max_output_tokens: u32,
    timeout: Duration,
}

impl AiClient {
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let api_key = env::var(API_KEY_ENV_VAR)
            .or_else(|_| env::var("OPENAI_API_KEY"))
            .map_err(|_| "AI API key is not configured")?;
        let model = env::var(MODEL_ENV_VAR).unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
        let max_output_tokens = env::var(MAX_OUTPUT_TOKENS_ENV_VAR)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2048);
        let timeout = env::var(TIMEOUT_ENV_VAR)
            .ok()
            .and_then(|value| value.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(60));

        let mut config = OpenAIConfig::new().with_api_key(api_key);
        if let Ok(api_base) = env::var(API_BASE_ENV_VAR) {
            config = config.with_api_base(api_base);
        }

        Ok(Self {
            client: Client::with_config(config),
            model,
            runtime: Runtime::new()?,
            max_output_tokens,
            timeout,
        })
    }

    pub fn respond(&self, instructions: &str, input: &str) -> Result<String, Box<dyn Error>> {
        let request = CreateResponseArgs::default()
            .model(&self.model)
            .instructions(instructions)
            .input(input)
            .max_output_tokens(self.max_output_tokens)
            .build()?;
        let response = self.runtime.block_on(async {
            tokio::time::timeout(self.timeout, self.client.responses().create(request)).await
        })??;
        response
            .output_text()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| "AI returned no text".into())
    }
}
