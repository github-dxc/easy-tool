use std::{path::Path, time::Duration};

use base64::Engine;
use chrono::{TimeZone, Utc};
use regex::Regex;
use serde_json::{Value, json};

use crate::settings::TencentCloudSettings;

use super::signing::{TencentSignatureInput, sign_v3};

const REGION: &str = "ap-guangzhou";
const TMT_HOST: &str = "tmt.tencentcloudapi.com";
const TMT_SERVICE: &str = "tmt";
const TMT_VERSION: &str = "2018-03-21";
const TMT_ACTION_TEXT_TRANSLATE: &str = "TextTranslate";
const OCR_HOST: &str = "ocr.tencentcloudapi.com";
const OCR_SERVICE: &str = "ocr";
const OCR_VERSION: &str = "2018-11-19";
const OCR_ACTION_GENERAL_BASIC: &str = "GeneralAccurateOCR";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const HTTP_ERROR_BODY_LIMIT: usize = 512;

pub enum TencentTranslationDirection {
    ZhToEn,
    EnToZh,
}

pub fn parse_text_translate_response(body: &str) -> Result<String, String> {
    let response = parse_response(body)?;
    if let Some(error) = response_error(&response) {
        return Err(error);
    }

    response
        .get("TargetText")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Tencent TextTranslate response missing TargetText".to_string())
}

pub fn parse_ocr_response(body: &str) -> Result<String, String> {
    let response = parse_response(body)?;
    if let Some(error) = response_error(&response) {
        return Err(error);
    }

    let detections = response
        .get("TextDetections")
        .and_then(Value::as_array)
        .ok_or_else(|| "Tencent OCR response missing TextDetections".to_string())?;
    let lines = detections
        .iter()
        .filter_map(|detection| detection.get("DetectedText").and_then(Value::as_str))
        .collect::<Vec<_>>();

    Ok(lines.join("\n"))
}

pub fn translate_text(
    credentials: &TencentCloudSettings,
    direction: TencentTranslationDirection,
    text: &str,
) -> Result<String, String> {
    let (source, target) = match direction {
        TencentTranslationDirection::ZhToEn => ("zh", "en"),
        TencentTranslationDirection::EnToZh => ("en", "zh"),
    };
    let payload = json!({
        "SourceText": text,
        "Source": source,
        "Target": target,
        "ProjectId": 0,
    })
    .to_string();

    let body = send_request(
        credentials,
        TMT_HOST,
        TMT_SERVICE,
        TMT_ACTION_TEXT_TRANSLATE,
        TMT_VERSION,
        &payload,
    )?;
    parse_text_translate_response(&body)
}

pub fn recognize_image(
    credentials: &TencentCloudSettings,
    image_path: &Path,
) -> Result<String, String> {
    let image = std::fs::read(image_path)
        .map_err(|err| format!("read image for Tencent OCR failed: {err}"))?;
    let image_base64 = base64::engine::general_purpose::STANDARD.encode(image);
    let payload = json!({
        "ImageBase64": image_base64,
    })
    .to_string();

    let body = send_request(
        credentials,
        OCR_HOST,
        OCR_SERVICE,
        OCR_ACTION_GENERAL_BASIC,
        OCR_VERSION,
        &payload,
    )?;
    parse_ocr_response(&body)
}

fn parse_response(body: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| format!("parse Tencent Cloud response failed: {err}"))?;
    value
        .get("Response")
        .cloned()
        .ok_or_else(|| "Tencent Cloud response missing Response".to_string())
}

fn response_error(response: &Value) -> Option<String> {
    let error = response.get("Error")?;
    let code = error
        .get("Code")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let message = error.get("Message").and_then(Value::as_str).unwrap_or("");
    if message.is_empty() {
        Some(format!("Tencent Cloud error: {code}"))
    } else {
        Some(format!("Tencent Cloud error {code}: {message}"))
    }
}

fn send_request(
    credentials: &TencentCloudSettings,
    host: &str,
    service: &str,
    action: &str,
    version: &str,
    payload: &str,
) -> Result<String, String> {
    let secret_id = credentials.secret_id.trim();
    let secret_key = credentials.secret_key.trim();
    if secret_id.is_empty() {
        return Err("Tencent Cloud secret_id is empty".to_string());
    }
    if secret_key.is_empty() {
        return Err("Tencent Cloud secret_key is empty".to_string());
    }

    let timestamp = Utc::now().timestamp();
    let date = Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .ok_or_else(|| "create Tencent Cloud signing timestamp failed".to_string())?
        .format("%Y-%m-%d")
        .to_string();
    let authorization = sign_v3(TencentSignatureInput {
        secret_id,
        secret_key,
        service,
        host,
        action,
        version,
        region: REGION,
        timestamp,
        date: &date,
        payload,
    });
    let timestamp_header = timestamp.to_string();
    let url = format!("https://{host}/");
    let response = ureq::post(&url)
        .set("Authorization", &authorization)
        .set("Content-Type", "application/json; charset=utf-8")
        .set("Host", host)
        .set("X-TC-Action", action)
        .set("X-TC-Version", version)
        .set("X-TC-Timestamp", &timestamp_header)
        .set("X-TC-Region", REGION)
        .timeout(REQUEST_TIMEOUT)
        .send_string(payload);

    match response {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("read Tencent Cloud response failed: {err}")),
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(format_http_status_error(status, &body))
        }
        Err(ureq::Error::Transport(err)) => Err(format!("Tencent Cloud request failed: {err}")),
    }
}

fn format_http_status_error(status: u16, body: &str) -> String {
    if let Ok(response) = parse_response(body) {
        if let Some(error) = response.get("Error") {
            let code = error
                .get("Code")
                .and_then(Value::as_str)
                .unwrap_or("Unknown");
            let message = error.get("Message").and_then(Value::as_str).unwrap_or("");
            let request_id = response.get("RequestId").and_then(Value::as_str);
            let mut formatted = if message.is_empty() {
                format!("Tencent Cloud request failed with HTTP {status}: {code}")
            } else {
                format!("Tencent Cloud request failed with HTTP {status}: {code}: {message}")
            };
            if let Some(request_id) = request_id {
                formatted.push_str(&format!(" (RequestId: {request_id})"));
            }
            return formatted;
        }
    }

    let sanitized = sanitize_http_error_body(body);
    if sanitized.is_empty() {
        format!("Tencent Cloud request failed with HTTP {status}")
    } else {
        format!("Tencent Cloud request failed with HTTP {status}: {sanitized}")
    }
}

fn sanitize_http_error_body(body: &str) -> String {
    let mut sanitized = body.replace(['\r', '\n'], " ");
    for pattern in [
        r"(?i)Authorization\s*[:=]\s*\S+",
        r"(?i)SecretId\s*[:=]\s*\S+",
        r"(?i)SecretKey\s*[:=]\s*\S+",
        r"(?i)Signature\s*[:=]\s*\S+",
        r"TC3-HMAC-SHA256\s+\S+",
    ] {
        sanitized = Regex::new(pattern)
            .expect("sensitive token redaction pattern should compile")
            .replace_all(&sanitized, "[redacted]")
            .to_string();
    }

    sanitized
        .chars()
        .take(HTTP_ERROR_BODY_LIMIT)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_translate_response_extracts_target_text() {
        let body = r#"{
            "Response": {
                "TargetText": "hello",
                "RequestId": "request-id"
            }
        }"#;

        assert_eq!(parse_text_translate_response(body), Ok("hello".to_string()));
    }

    #[test]
    fn parse_ocr_response_joins_detected_text_in_order() {
        let body = r#"{
            "Response": {
                "TextDetections": [
                    { "DetectedText": "first" },
                    { "DetectedText": "second" },
                    { "DetectedText": "third" }
                ],
                "RequestId": "request-id"
            }
        }"#;

        assert_eq!(
            parse_ocr_response(body),
            Ok("first\nsecond\nthird".to_string())
        );
    }

    #[test]
    fn text_translate_error_response_returns_error_containing_code() {
        let body = r#"{
            "Response": {
                "Error": {
                    "Code": "AuthFailure.SignatureFailure",
                    "Message": "signature failed"
                },
                "RequestId": "request-id"
            }
        }"#;

        let err = parse_text_translate_response(body).expect_err("error response should fail");
        assert!(err.contains("AuthFailure.SignatureFailure"));
    }

    #[test]
    fn ocr_error_response_returns_error_containing_code() {
        let body = r#"{
            "Response": {
                "Error": {
                    "Code": "FailedOperation.OcrFailed",
                    "Message": "ocr failed"
                },
                "RequestId": "request-id"
            }
        }"#;

        let err = parse_ocr_response(body).expect_err("error response should fail");
        assert!(err.contains("FailedOperation.OcrFailed"));
    }

    #[test]
    fn http_error_formatter_extracts_tencent_error_without_full_body() {
        let body = r#"{
            "Response": {
                "Error": {
                    "Code": "AuthFailure.SignatureFailure",
                    "Message": "signature failed"
                },
                "RequestId": "request-id"
            }
        }"#;

        assert_eq!(
            format_http_status_error(401, body),
            "Tencent Cloud request failed with HTTP 401: AuthFailure.SignatureFailure: signature failed (RequestId: request-id)"
        );
    }

    #[test]
    fn http_error_formatter_sanitizes_and_caps_non_tencent_body() {
        let body = format!(
            "line1\nAuthorization: should-not-be-echoed SecretId=abc SecretKey=def Signature=ghi\n{}",
            "x".repeat(700)
        );
        let formatted = format_http_status_error(500, &body);

        assert!(
            formatted.starts_with("Tencent Cloud request failed with HTTP 500: line1 [redacted]")
        );
        assert!(formatted.len() < 590);
        assert!(!formatted.contains('\n'));
        assert!(!formatted.contains("should-not-be-echoed"));
        assert!(!formatted.contains("SecretId=abc"));
        assert!(!formatted.contains("SecretKey=def"));
        assert!(!formatted.contains("Signature=ghi"));
        assert!(!formatted.contains(&"x".repeat(600)));
    }
}
