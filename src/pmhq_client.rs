//! PMHQ HTTP API 客户端

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct PMHQClient {
    base_url: String,
    timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct SelfInfo {
    pub uin: String,
    pub nickname: String,
}

#[derive(Serialize)]
struct CallRequest {
    r#type: &'static str,
    data: CallData,
}

#[derive(Serialize)]
struct CallData {
    func: &'static str,
    args: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct CallResponse {
    r#type: String,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct SSEData {
    r#type: Option<String>,
    data: Option<serde_json::Value>,
}

impl PMHQClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{}", port),
            timeout: Duration::from_secs(5),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn call(&self, func: &'static str) -> Result<serde_json::Value, String> {
        let payload = CallRequest {
            r#type: "call",
            data: CallData { func, args: vec![] },
        };

        let body_str =
            serde_json::to_string(&payload).map_err(|e| format!("序列化失败: {}", e))?;

        let resp = ureq::post(&self.base_url)
            .timeout(self.timeout)
            .set("Content-Type", "application/json")
            .send_string(&body_str)
            .map_err(|e| format!("请求失败: {}", e))?;

        let resp_str = resp
            .into_string()
            .map_err(|e| format!("读取响应失败: {}", e))?;

        let body: CallResponse =
            serde_json::from_str(&resp_str).map_err(|e| format!("解析响应失败: {}", e))?;

        if body.r#type != "call" {
            return Err("响应类型错误".to_string());
        }

        let inner: serde_json::Value = if body.data.is_string() {
            serde_json::from_str(body.data.as_str().unwrap())
                .map_err(|e| format!("解析内部数据失败: {}", e))?
        } else {
            body.data
        };

        if let Some(result) = inner.get("result") {
            if let Some(s) = result.as_str() {
                if s.contains("Error") {
                    return Err(s.to_string());
                }
            }
            Ok(result.clone())
        } else {
            Err("响应缺少 result 字段".to_string())
        }
    }

    pub fn get_self_info(&self) -> Result<SelfInfo, String> {
        let result = self.call("getSelfInfo")?;

        let uin = result
            .get("uin")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .or_else(|| {
                result
                    .get("uin")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.to_string())
            })
            .unwrap_or_default();

        let nickname = result
            .get("nickName")
            .or_else(|| result.get("nickname"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if uin.is_empty() {
            return Err("未获取到 QQ 号".to_string());
        }

        Ok(SelfInfo { uin, nickname })
    }

    pub fn request_qrcode(&self) -> Result<(), String> {
        let payload = CallRequest {
            r#type: "call",
            data: CallData {
                func: "loginService.getQRCodePicture",
                args: vec![],
            },
        };

        let body_str =
            serde_json::to_string(&payload).map_err(|e| format!("序列化失败: {}", e))?;

        ureq::post(&self.base_url)
            .timeout(self.timeout)
            .set("Content-Type", "application/json")
            .send_string(&body_str)
            .map_err(|e| format!("请求二维码失败: {}", e))?;

        Ok(())
    }

    /// 启动 SSE 监听，处理二维码和登录事件
    pub fn start_sse_listener<F>(&self, logged_in: Arc<AtomicBool>, mut on_qrcode: F)
    where
        F: FnMut(&str, &str) + Send + 'static,
    {
        let url = self.base_url.clone();

        loop {
            if logged_in.load(Ordering::Relaxed) {
                break;
            }

            match ureq::get(&url)
                .timeout(Duration::from_secs(300))
                .set("Accept", "text/event-stream")
                .call()
            {
                Ok(resp) => {
                    let reader = BufReader::new(resp.into_reader());
                    for line in reader.lines() {
                        if logged_in.load(Ordering::Relaxed) {
                            return;
                        }

                        let line = match line {
                            Ok(l) => l,
                            Err(_) => break,
                        };

                        if !line.starts_with("data: ") {
                            continue;
                        }

                        let json_str = &line[6..];
                        if let Ok(data) = serde_json::from_str::<SSEData>(json_str) {
                            // 处理二维码事件
                            if data.r#type.as_deref() == Some("nodeIKernelLoginListener") {
                                if let Some(inner) = &data.data {
                                    if inner.get("sub_type").and_then(|v| v.as_str())
                                        == Some("onQRCodeGetPicture")
                                    {
                                        if let Some(qr_data) =
                                            inner.get("data").and_then(|d| d.as_object())
                                        {
                                            let png_base64 = qr_data
                                                .get("pngBase64QrcodeData")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            let qrcode_url = qr_data
                                                .get("qrcodeUrl")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");

                                            if !qrcode_url.is_empty() {
                                                on_qrcode(qrcode_url, png_base64);
                                            }
                                        }
                                    }
                                }
                            }

                            // 处理登录成功事件
                            if data.r#type.as_deref()
                                == Some("nodeIQQNTWrapperSessionListener")
                            {
                                if let Some(inner) = &data.data {
                                    if inner.get("sub_type").and_then(|v| v.as_str())
                                        == Some("onSessionInitComplete")
                                    {
                                        logged_in.store(true, Ordering::Relaxed);
                                        return;
                                    }
                                }
                            }

                            // 处理 account_ready 事件
                            if data.r#type.as_deref() == Some("account_ready") {
                                logged_in.store(true, Ordering::Relaxed);
                                return;
                            }
                        }
                    }
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    }
}
