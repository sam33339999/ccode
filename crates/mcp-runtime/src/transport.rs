use std::io;
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[derive(Debug, thiserror::Error)]
pub enum ConnectionFailure {
    #[error("failed to spawn MCP server process `{program}`")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("MCP server process `{program}` did not provide stdin pipe")]
    MissingStdin { program: String },
    #[error("MCP server process `{program}` did not provide stdout pipe")]
    MissingStdout { program: String },
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error(transparent)]
    Connection(#[from] ConnectionFailure),
    #[error("failed to serialize JSON-RPC message")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to deserialize JSON-RPC message")]
    Deserialize(#[source] serde_json::Error),
    #[error("I/O error")]
    Io(#[from] io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("transport is closed")]
    Closed,
}

#[async_trait]
pub trait McpTransport: Send {
    async fn send(&mut self, message: &Value) -> Result<(), TransportError>;
    async fn receive(&mut self) -> Result<Value, TransportError>;
    async fn close(&mut self) -> Result<(), TransportError>;
}

pub struct StdioTransport {
    program: String,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    closed: bool,
}

impl StdioTransport {
    pub async fn spawn(program: &str, args: &[&str]) -> Result<Self, TransportError> {
        let mut command = Command::new(program);
        command.args(args);
        Self::from_command(program.to_owned(), command).await
    }

    pub async fn from_command(
        program: String,
        mut command: Command,
    ) -> Result<Self, TransportError> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped());

        let mut child = command.spawn().map_err(|source| ConnectionFailure::Spawn {
            program: program.clone(),
            source,
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ConnectionFailure::MissingStdin {
                program: program.clone(),
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ConnectionFailure::MissingStdout {
                program: program.clone(),
            })?;

        Ok(Self {
            program,
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout).lines(),
            closed: false,
        })
    }

    fn ensure_open(&self) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        Ok(())
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&mut self, message: &Value) -> Result<(), TransportError> {
        self.ensure_open()?;

        let line = encode_jsonrpc_line(message)?;
        let stdin = self.stdin.as_mut().ok_or(TransportError::Closed)?;
        stdin.write_all(&line).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<Value, TransportError> {
        self.ensure_open()?;

        match self.stdout.next_line().await? {
            Some(line) => decode_jsonrpc_line(&line),
            None => {
                self.closed = true;
                match self.child.try_wait()? {
                    Some(status) => Err(TransportError::Protocol(format!(
                        "MCP server process `{}` exited before delivering a complete JSON-RPC message (status: {})",
                        self.program, status
                    ))),
                    None => Err(TransportError::Closed),
                }
            }
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if self.closed {
            return Ok(());
        }

        self.closed = true;

        if let Some(mut stdin) = self.stdin.take() {
            stdin.shutdown().await?;
        }

        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }

        let _ = self.child.wait().await?;
        Ok(())
    }
}

pub(crate) fn encode_jsonrpc_line(message: &Value) -> Result<Vec<u8>, TransportError> {
    let mut bytes = serde_json::to_vec(message).map_err(TransportError::Serialize)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn decode_jsonrpc_line(line: &str) -> Result<Value, TransportError> {
    if line.trim().is_empty() {
        return Err(TransportError::Protocol(
            "received empty JSON-RPC message line".to_owned(),
        ));
    }

    serde_json::from_str(line).map_err(TransportError::Deserialize)
}
