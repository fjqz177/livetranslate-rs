//! worker IPC 帧协议（PLAN §3.2，对齐原版 asr_worker 的 JSON 语义）。
//!
//! 帧 = [u32 payload_len（LE）][payload]；payload = JSON + （transcribe 请求时
//! 紧随的 f32 LE PCM）。JSON 与音频同帧，worker 按 JSON 后剩余字节取音频。
//!
//! 响应首帧 ready（id=null）；错误响应带 `{"message","recoverable"}`。

use lt_proto::AsrResult;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// 帧大小上限（防脏流撑爆内存；8s 音频 f32 ≈ 2MB，远低于此）
pub const MAX_FRAME_BYTES: u32 = 256 * 1024 * 1024;

/// 请求类型（`{"id","type":...}` 中的 type 载荷）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ReqKind {
    Transcribe {
        #[serde(default)]
        word_timestamps: bool,
    },
    SetLanguage {
        language: String,
    },
    SetInputPadding {
        pad_seconds: f32,
    },
    Shutdown,
}

/// 请求帧的 JSON 部分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    #[serde(flatten)]
    pub kind: ReqKind,
}

/// ready 载荷（对应原版 ready payload：engine/display_name）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadyInfo {
    pub engine: String,
    pub display_name: String,
}

/// 错误载荷（recoverable 语义：加载失败=不可恢复；单命令错误=可恢复）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorInfo {
    pub message: String,
    #[serde(default = "default_recoverable")]
    pub recoverable: bool,
}

fn default_recoverable() -> bool {
    true
}

/// 响应类型（`{"id","ok","type":...}` 中的 type 载荷）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RespKind {
    Ready(ReadyInfo),
    Result(AsrResult),
    Ack,
    Shutdown,
    Error(ErrorInfo),
}

/// 响应帧的 JSON 部分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: Option<String>,
    pub ok: bool,
    #[serde(flatten)]
    pub kind: RespKind,
}

impl Response {
    pub fn ready(info: ReadyInfo) -> Self {
        Self {
            id: None,
            ok: true,
            kind: RespKind::Ready(info),
        }
    }
    pub fn result(id: &str, result: AsrResult) -> Self {
        Self {
            id: Some(id.into()),
            ok: true,
            kind: RespKind::Result(result),
        }
    }
    pub fn ack(id: &str) -> Self {
        Self {
            id: Some(id.into()),
            ok: true,
            kind: RespKind::Ack,
        }
    }
    pub fn shutdown(id: &str) -> Self {
        Self {
            id: Some(id.into()),
            ok: true,
            kind: RespKind::Shutdown,
        }
    }
    pub fn error(id: Option<&str>, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            id: id.map(Into::into),
            ok: false,
            kind: RespKind::Error(ErrorInfo {
                message: message.into(),
                recoverable,
            }),
        }
    }

    /// 便捷取错误信息（ok=false 时）
    pub fn error_info(&self) -> Option<&ErrorInfo> {
        match &self.kind {
            RespKind::Error(e) => Some(e),
            _ => None,
        }
    }
}

/// 帧写入（父子两端共用）
pub struct FrameWriter<W: Write> {
    inner: W,
    /// 音频字节 staging（帧间复用免重分配；AH-10/H20：整块单次写替代
    /// 逐样本 write_all——裸管道上每 4 字节样本一次 syscall，8s 段 ≈12.8 万次）
    audio_buf: Vec<u8>,
}

impl<W: Write> FrameWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            audio_buf: Vec::new(),
        }
    }

    fn write_frame(&mut self, json: &[u8], audio: &[f32]) -> std::io::Result<()> {
        let total = json.len() as u64 + audio.len() as u64 * 4;
        u32::try_from(total)
            .ok()
            .filter(|&n| n <= MAX_FRAME_BYTES)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "帧超限"))?;
        self.inner.write_all(&(total as u32).to_le_bytes())?;
        self.inner.write_all(json)?;
        self.audio_buf.clear();
        self.audio_buf.reserve(audio.len() * 4);
        for s in audio {
            self.audio_buf.extend_from_slice(&s.to_le_bytes());
        }
        self.inner.write_all(&self.audio_buf)?;
        self.inner.flush()
    }

    /// worker/客户端通用：发请求（transcribe 携带音频）
    pub fn write_request(&mut self, req: &Request, audio: &[f32]) -> std::io::Result<()> {
        let json = serde_json::to_vec(req)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.write_frame(&json, audio)
    }

    /// 发响应（无音频）
    pub fn write_response(&mut self, resp: &Response) -> std::io::Result<()> {
        let json = serde_json::to_vec(resp)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.write_frame(&json, &[])
    }
}

/// 一条完整请求（JSON + 解出的音频）
pub struct IncomingRequest {
    pub request: Request,
    pub audio: Vec<f32>,
}

/// 帧读取（父子两端共用）；流关闭（EOF 且无半帧）返回 None
pub struct FrameReader<R: Read> {
    inner: R,
}

impl<R: Read> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner }
    }

    fn read_exact_opt(&mut self, buf: &mut [u8]) -> std::io::Result<bool> {
        let mut off = 0;
        while off < buf.len() {
            match self.inner.read(&mut buf[off..]) {
                Ok(0) => {
                    return if off == 0 {
                        Ok(false) // 干净 EOF
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "半帧 EOF",
                        ))
                    };
                }
                Ok(n) => off += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// 读一帧原始 payload（JSON + 可选音频尾巴）
    fn read_raw_frame(&mut self) -> std::io::Result<Option<(Vec<u8>, Vec<f32>)>> {
        let mut len_buf = [0u8; 4];
        if !self.read_exact_opt(&mut len_buf)? {
            return Ok(None);
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len as u32 > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("帧长 {len} 超限"),
            ));
        }
        let mut payload = vec![0u8; len];
        self.read_exact_opt(&mut payload)?;
        // 尾部按 f32 LE 解码：调用方约定 transcribe 请求的 JSON 后是原始 PCM。
        // 本层不知道 JSON 边界，交由 parse_request 按 JSON 实际长度切分。
        Ok(Some((payload, Vec::new())))
    }

    /// worker 侧：读一条请求（含音频切分）
    pub fn read_request(&mut self) -> std::io::Result<Option<IncomingRequest>> {
        let Some((payload, _)) = self.read_raw_frame()? else {
            return Ok(None);
        };
        let (req, audio_bytes) = split_request_payload(&payload)?;
        let audio = audio_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Ok(Some(IncomingRequest {
            request: req,
            audio,
        }))
    }

    /// 客户端侧：读一条响应（无音频尾巴）
    pub fn read_response(&mut self) -> std::io::Result<Option<Response>> {
        let Some((payload, _)) = self.read_raw_frame()? else {
            return Ok(None);
        };
        serde_json::from_slice(&payload)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// 请求 payload 切分：JSON 对象结束处之后的字节即音频。
/// 用 serde_json 的增量切断点（Span 不适用，这里用保守的括号配平，
/// JSON 字符串内的引号/转义由 serde_json::Deserializer 处理）。
fn split_request_payload(payload: &[u8]) -> std::io::Result<(Request, &[u8])> {
    // 先用 serde_json 流式定位 JSON 结束偏移（避免自己处理转义）
    let mut de = serde_json::Deserializer::from_slice(payload).into_iter::<Request>();
    let req = match de.next() {
        Some(Ok(r)) => r,
        Some(Err(e)) => {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
        }
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "空请求",
            ));
        }
    };
    let json_end = de.byte_offset();
    Ok((req, &payload[json_end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn response_roundtrip() {
        let resp = Response::result(
            "abc123",
            AsrResult {
                text: "你好世界".into(),
                language: "zh".into(),
                language_name: "zh".into(),
                words: None,
            },
        );
        let mut w = FrameWriter::new(Vec::new());
        w.write_response(&resp).unwrap();
        let bytes = w.inner;
        let mut r = FrameReader::new(Cursor::new(bytes));
        let got = r.read_response().unwrap().unwrap();
        assert_eq!(got.id.as_deref(), Some("abc123"));
        assert!(got.ok);
        match got.kind {
            RespKind::Result(res) => assert_eq!(res.text, "你好世界"),
            _ => panic!("类型错误"),
        }
        // EOF → None
        assert!(r.read_response().unwrap().is_none());
    }

    #[test]
    fn request_with_audio_roundtrip() {
        let req = Request {
            id: "xyz".into(),
            kind: ReqKind::Transcribe {
                word_timestamps: false,
            },
        };
        let audio: Vec<f32> = (0..512).map(|i| i as f32 * 0.01).collect();
        let mut w = FrameWriter::new(Vec::new());
        w.write_request(&req, &audio).unwrap();
        let bytes = w.inner;
        let mut r = FrameReader::new(Cursor::new(bytes));
        let inc = r.read_request().unwrap().unwrap();
        assert_eq!(inc.request.id, "xyz");
        assert_eq!(inc.audio.len(), 512);
        assert_eq!(inc.audio[100], 1.0);
        assert!(r.read_request().unwrap().is_none());
    }

    #[test]
    fn request_without_audio() {
        for kind in [
            ReqKind::SetLanguage {
                language: "zh".into(),
            },
            ReqKind::SetInputPadding { pad_seconds: 0.5 },
            ReqKind::Shutdown,
        ] {
            let req = Request {
                id: "a".into(),
                kind,
            };
            let mut w = FrameWriter::new(Vec::new());
            w.write_request(&req, &[]).unwrap();
            let mut r = FrameReader::new(Cursor::new(w.inner));
            let inc = r.read_request().unwrap().unwrap();
            assert_eq!(inc.audio.len(), 0);
            assert_eq!(inc.request.id, "a");
        }
    }

    #[test]
    fn half_frame_eof_errors() {
        let req = Request {
            id: "a".into(),
            kind: ReqKind::Shutdown,
        };
        let mut w = FrameWriter::new(Vec::new());
        w.write_request(&req, &[]).unwrap();
        let mut bytes = w.inner;
        bytes.pop(); // 掐掉最后一字节
        let mut r = FrameReader::new(Cursor::new(bytes));
        assert!(r.read_request().is_err());
    }

    #[test]
    fn ready_and_error_shapes() {
        let ready = Response::ready(ReadyInfo {
            engine: "sensevoice".into(),
            display_name: "SenseVoice Small".into(),
        });
        let js = serde_json::to_value(&ready).unwrap();
        assert_eq!(js["id"], serde_json::Value::Null);
        assert_eq!(js["type"], "ready");

        let err = Response::error(Some("e1"), "boom", false);
        assert!(!err.ok);
        assert_eq!(err.error_info().unwrap().recoverable, false);
    }
}
