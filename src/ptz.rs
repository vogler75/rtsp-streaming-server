use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::Utc;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use sha1::{Sha1, Digest};

use crate::errors::Result;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PtzVelocity {
    pub pan: f32,  // -1.0..1.0 left/right
    pub tilt: f32, // -1.0..1.0 down/up
    pub zoom: f32, // -1.0..1.0 out/in
}

impl Default for PtzVelocity {
    fn default() -> Self { Self { pan: 0.0, tilt: 0.0, zoom: 0.0 } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtzPresetRequest {
    pub name: Option<String>,
    pub token: Option<String>,
}

#[async_trait]
pub trait PtzController: Send + Sync {
    async fn continuous_move(&self, velocity: PtzVelocity, timeout_secs: Option<u64>) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn goto_preset(&self, preset_token: &str, speed: Option<PtzVelocity>) -> Result<()>;
    async fn set_preset(&self, req: PtzPresetRequest) -> Result<String>; // returns preset token
}

pub mod onvif_ptz {
    use super::*;
    use crate::errors::StreamError;
    use tracing::{debug, trace};

    #[derive(Clone)]
    pub struct OnvifPtz {
        pub endpoint: String,
        pub username: Option<String>,
        pub password: Option<String>,
        pub profile_token: String,
        client: reqwest::Client,
    }

    impl OnvifPtz {
        pub fn new(endpoint: String, username: Option<String>, password: Option<String>, profile_token: String) -> Self {
            let client = reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .expect("failed to build http client");
            Self { endpoint, username, password, profile_token, client }
        }

        fn soap_envelope_with_wsse(&self, body: &str) -> String {
            let header = self.wsse_header();
            if let Some(h) = header {
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                     <s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\"\n\
                      xmlns:tt=\"http://www.onvif.org/ver10/schema\"\n\
                      xmlns:tptz=\"http://www.onvif.org/ver20/ptz/wsdl\">\n\
                       <s:Header>{}</s:Header>\n\
                       <s:Body>{}</s:Body>\n\
                     </s:Envelope>",
                    h, body
                )
            } else {
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                     <s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\"\n\
                      xmlns:tt=\"http://www.onvif.org/ver10/schema\"\n\
                      xmlns:tptz=\"http://www.onvif.org/ver20/ptz/wsdl\">\n\
                       <s:Body>{}</s:Body>\n\
                     </s:Envelope>",
                    body
                )
            }
        }

        fn wsse_header(&self) -> Option<String> {
            let (username, password) = match (&self.username, &self.password) {
                (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => (u.clone(), p.clone()),
                _ => return None,
            };
            // Build WS-Security UsernameToken with PasswordDigest
            let nonce_bytes = *Uuid::new_v4().as_bytes();
            let nonce_b64 = B64.encode(nonce_bytes);
            let created = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let mut hasher = Sha1::new();
            hasher.update(nonce_bytes);
            hasher.update(created.as_bytes());
            hasher.update(password.as_bytes());
            let digest = hasher.finalize();
            let pwd_digest_b64 = B64.encode(digest);

            let header = format!(
                "<wsse:Security s:mustUnderstand=\"1\"\n\
                    xmlns:wsse=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd\"\n\
                    xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\">\n\
                    <wsse:UsernameToken>\n\
                        <wsse:Username>{}</wsse:Username>\n\
                        <wsse:Password Type=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest\">{}</wsse:Password>\n\
                        <wsse:Nonce EncodingType=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary\">{}</wsse:Nonce>\n\
                        <wsu:Created>{}</wsu:Created>\n\
                    </wsse:UsernameToken>\n\
                </wsse:Security>",
                xml_escape(&username), pwd_digest_b64, nonce_b64, created
            );
            Some(header)
        }

        async fn post(&self, action: &str, body: String) -> Result<String> {
            // Avoid logging credentials; include endpoint and action only
            debug!(target: "ptz_onvif", action = action, endpoint = %self.endpoint, "Sending ONVIF request");
            let mut req = self.client.post(&self.endpoint)
                .header("Content-Type", "application/soap+xml; charset=utf-8")
                .header("SOAPAction", action)
                .body(body);
            if let (Some(u), Some(p)) = (&self.username, &self.password) {
                req = req.basic_auth(u, Some(p));
            }
            let res = req.send().await.map_err(|e| {
                debug!(target: "ptz_onvif", action = action, endpoint = %self.endpoint, error = %e, "ONVIF HTTP error");
                StreamError::server(format!("ONVIF PTZ HTTP error: {}", e))
            })?;
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            debug!(target: "ptz_onvif", action = action, endpoint = %self.endpoint, status = %status, resp_len = text.len(), "ONVIF response received");
            trace!(target: "ptz_onvif", action = action, endpoint = %self.endpoint, response = %text, "ONVIF response body");
            if !status.is_success() {
                return Err(StreamError::server(format!("ONVIF PTZ bad status {}: {}", status, text)).into());
            }
            Ok(text)
        }
    }

    #[async_trait]
    impl PtzController for OnvifPtz {
        async fn continuous_move(&self, velocity: PtzVelocity, timeout_secs: Option<u64>) -> Result<()> {
            debug!(target: "ptz_onvif", endpoint = %self.endpoint, profile = %self.profile_token, pan = velocity.pan, tilt = velocity.tilt, zoom = velocity.zoom, timeout = ?timeout_secs, "ONVIF ContinuousMove");
            let body = format!(
                "<tptz:ContinuousMove>\n\
                    <tptz:ProfileToken>{}</tptz:ProfileToken>\n\
                    <tptz:Velocity>\n\
                        <tt:PanTilt x=\"{}\" y=\"{}\"/>\n\
                        <tt:Zoom x=\"{}\"/>\n\
                    </tptz:Velocity>\n\
                    {}\n\
                 </tptz:ContinuousMove>",
                self.profile_token,
                velocity.pan, velocity.tilt, velocity.zoom,
                timeout_secs.map(|t| format!("<tptz:Timeout>PT{}S</tptz:Timeout>", t)).unwrap_or_default()
            );
            let env = self.soap_envelope_with_wsse(&body);
            let _ = self.post("http://www.onvif.org/ver20/ptz/wsdl/ContinuousMove", env).await?;
            Ok(())
        }

        async fn stop(&self) -> Result<()> {
            debug!(target: "ptz_onvif", endpoint = %self.endpoint, profile = %self.profile_token, "ONVIF Stop");
            let body = format!(
                "<tptz:Stop>\n\
                    <tptz:ProfileToken>{}</tptz:ProfileToken>\n\
                    <tptz:PanTilt>true</tptz:PanTilt>\n\
                    <tptz:Zoom>true</tptz:Zoom>\n\
                 </tptz:Stop>",
                self.profile_token
            );
            let env = self.soap_envelope_with_wsse(&body);
            let _ = self.post("http://www.onvif.org/ver20/ptz/wsdl/Stop", env).await?;
            Ok(())
        }

        async fn goto_preset(&self, preset_token: &str, _speed: Option<PtzVelocity>) -> Result<()> {
            debug!(target: "ptz_onvif", endpoint = %self.endpoint, profile = %self.profile_token, preset = preset_token, "ONVIF GotoPreset");
            let body = format!(
                "<tptz:GotoPreset>\n\
                    <tptz:ProfileToken>{}</tptz:ProfileToken>\n\
                    <tptz:PresetToken>{}</tptz:PresetToken>\n\
                 </tptz:GotoPreset>",
                self.profile_token, preset_token
            );
            let env = self.soap_envelope_with_wsse(&body);
            let _ = self.post("http://www.onvif.org/ver20/ptz/wsdl/GotoPreset", env).await?;
            Ok(())
        }

        async fn set_preset(&self, req: PtzPresetRequest) -> Result<String> {
            debug!(target: "ptz_onvif", endpoint = %self.endpoint, profile = %self.profile_token, name = %req.name.as_deref().unwrap_or(""), token = %req.token.as_deref().unwrap_or(""), "ONVIF SetPreset");
            let name_xml = req.name.as_ref().map(|n| format!("<tptz:PresetName>{}</tptz:PresetName>", xml_escape(n))).unwrap_or_default();
            let token_xml = req.token.as_ref().map(|t| format!("<tptz:PresetToken>{}</tptz:PresetToken>", xml_escape(t))).unwrap_or_default();
            let body = format!(
                "<tptz:SetPreset>\n\
                    <tptz:ProfileToken>{}</tptz:ProfileToken>\n\
                    {}{}\n\
                 </tptz:SetPreset>",
                self.profile_token, name_xml, token_xml
            );
            let env = self.soap_envelope_with_wsse(&body);
            let resp = self.post("http://www.onvif.org/ver20/ptz/wsdl/SetPreset", env).await?;
            if let Some(start) = resp.find("<tptz:PresetToken>") {
                if let Some(end_rel) = resp[start..].find("</tptz:PresetToken>") {
                    let token = &resp[start + 17..start + end_rel];
                    return Ok(token.to_string());
                }
            }
            Ok(String::new())
        }
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}
