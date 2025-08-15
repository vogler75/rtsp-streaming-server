use thiserror::Error;

#[derive(Error, Debug)]
pub enum StreamError {
    #[error("Configuration error: {message}")]
    Config { message: String },
    
    #[error("RTSP connection failed: {message}")]
    RtspConnection { message: String },
    
    
    #[error("MQTT error: {message}")]
    Mqtt { message: String },
    
    #[error("FFmpeg error: {message}")]
    Ffmpeg { message: String },
    
    
    #[error("Server error: {message}")]
    Server { message: String },
    
    #[error("IO error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },
    
    #[error("URL parse error: {source}")]
    UrlParse {
        #[from]
        source: url::ParseError,
    },
    
    #[error("TOML parse error: {source}")]
    TomlParse {
        #[from]
        source: toml::de::Error,
    },
    
    #[error("JSON error: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    
    #[error("Network address parse error: {source}")]
    AddrParse {
        #[from]
        source: std::net::AddrParseError,
    },
    
    #[error("MQTT client error: {source}")]
    MqttClient {
        #[from]
        source: rumqttc::ClientError,
    },
}

impl StreamError {
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config { message: message.into() }
    }
    
    pub fn rtsp_connection(message: impl Into<String>) -> Self {
        Self::RtspConnection { message: message.into() }
    }
    
    
    pub fn mqtt(message: impl Into<String>) -> Self {
        Self::Mqtt { message: message.into() }
    }
    
    pub fn ffmpeg(message: impl Into<String>) -> Self {
        Self::Ffmpeg { message: message.into() }
    }
    
    
    pub fn server(message: impl Into<String>) -> Self {
        Self::Server { message: message.into() }
    }
}

pub type Result<T> = std::result::Result<T, StreamError>;