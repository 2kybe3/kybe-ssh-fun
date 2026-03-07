use std::{borrow::Cow, collections::HashMap, fs, path::Path, sync::Arc};

use russh::{
    ChannelId, CryptoVec, Pty,
    keys::{PrivateKey, ssh_key::rand_core::OsRng},
    server::{self, Server as _},
};
use tokio::{net::TcpListener, sync::Mutex};
use tracing::{info, warn};

const PGP_KEY: &str = include_str!("../static/pgp.txt");
const IDENTITY: &str = include_str!("../static/identity.txt");

#[derive(Clone, Debug)]
#[allow(unused)]
struct ClientState {
    channel: ChannelId,
    handle: russh::server::Handle,
    buffer: String,
    cursor: usize,
    escape: Vec<u8>,
}

impl ClientState {
    pub fn new(channel: ChannelId, handle: russh::server::Handle) -> Self {
        Self {
            channel,
            handle,
            buffer: String::default(),
            cursor: usize::default(),
            escape: Vec::default(),
        }
    }
}

#[derive(Clone)]
struct Server {
    clients: Arc<Mutex<HashMap<ChannelId, ClientState>>>,
    id: usize,
}

impl server::Server for Server {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        let s = self.clone();
        self.id += 1;
        s
    }
    fn handle_session_error(&mut self, error: <Self::Handler as server::Handler>::Error) {
        warn!(error = ?error, "session_error");
    }
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data);
        info!(channel = ?channel, cmd = ?cmd, "exec_request");

        let args: Vec<&str> = cmd.split(" ").collect();
        if let Some(main) = args.first() {
            match *main {
                "ident" | "identity" | "who" => {
                    session.data(channel, CryptoVec::from(IDENTITY))?;
                    session.close(channel)?;
                }
                "pgp" | "gpg" => {
                    session.data(channel, CryptoVec::from(PGP_KEY))?;
                    session.close(channel)?;
                }
                _ => {
                    session.data(
                        channel,
                        CryptoVec::from(format!(
                            "get that dirty \"{}\" away from me, try \"ident, pgp\"\n",
                            cmd
                        )),
                    )?;
                    session.close(channel)?;
                }
            }
        }
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        pix_width: u32,
        pix_height: u32,
        modes: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        info!(channel = ?channel, term = ?term, col_width = ?col_width, row_height = ?row_height, pix_width = ?pix_width, pix_height = ?pix_height, modes = ?modes, "pty_request");

        let mut output = Vec::new();
        {
            let mut clients = self.clients.lock().await;
            let state = match clients.get_mut(&channel) {
                Some(s) => s,
                None => return Ok(()),
            };
            redraw_line(state, &mut output);
        }

        for chunk in output {
            session.data(channel, chunk)?;
        }

        session.channel_success(channel)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let mut output = Vec::new();
        let mut should_close = false;

        {
            let mut clients = self.clients.lock().await;
            let state = match clients.get_mut(&channel) {
                Some(s) => s,
                None => return Ok(()),
            };

            for &byte in data {
                if byte == 0x1b || !state.escape.is_empty() {
                    state.escape.push(byte);
                    handle_escape(state, &mut output);
                    continue;
                }
                match byte {
                    b'\r' | b'\n' => {
                        state.cursor = 0;
                        let input = state.buffer.trim().to_string();
                        state.buffer.clear();

                        output.push(CryptoVec::from("\r\n"));

                        match input.as_str() {
                            "help" => output.push(CryptoVec::from(
                                "Commands: ident, pgp, ping, clear, help, exit\r\n",
                            )),
                            "ident" | "identity" | "who" => output.push(CryptoVec::from(format!(
                                "{}\r\n",
                                IDENTITY.replace("\n", "\r\n")
                            ))),
                            "gpg" | "pgp" => output.push(CryptoVec::from(format!(
                                "{}\r\n",
                                PGP_KEY.replace("\n", "\r\n")
                            ))),
                            "ping" => output.push(CryptoVec::from("pong\r\n")),
                            "clear" => output.push(CryptoVec::from("\x1b[2J\x1b[H")),
                            "exit" => should_close = true,
                            "" => {}
                            _ => output.push(CryptoVec::from("Unknown command\r\n")),
                        }

                        if !should_close {
                            output.push(CryptoVec::from("> "));
                        }
                    }

                    // CTRL+A
                    1 => {
                        state.cursor = 0;
                        redraw_line(state, &mut output);
                    }

                    // Ctrl+E
                    5 => {
                        state.cursor = state.buffer.len();
                        redraw_line(state, &mut output);
                    }

                    3 => {
                        should_close = true;
                    }

                    127 => {
                        if !state.buffer.is_empty() && state.cursor > 0 {
                            state.cursor -= 1;
                            state.buffer.remove(state.cursor);
                            redraw_line(state, &mut output);
                        }
                    }

                    byte if byte.is_ascii_graphic() || byte == b' ' => {
                        state.cursor = state.cursor.min(state.buffer.len());
                        state.buffer.insert(state.cursor, byte as char);
                        state.cursor += 1;
                        redraw_line(state, &mut output);
                    }

                    _ => {}
                }
            }
        }

        for chunk in output {
            session.data(channel, chunk)?;
        }

        if should_close {
            session.close(channel)?;
        }

        Ok(())
    }

    async fn channel_open_session(
        &mut self,
        channel: russh::Channel<server::Msg>,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        {
            let mut clients = self.clients.lock().await;
            clients.insert(
                channel.id(),
                ClientState::new(channel.id(), session.handle()),
            );
        }
        Ok(true)
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let mut clients = self.clients.lock().await;
        clients.remove(&channel);
        Ok(())
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<server::Auth, Self::Error> {
        if user.contains("tilley") {
            if let Some(resp) = response {
                let mut results = Vec::new();

                for item in resp {
                    results.push(item.to_vec());
                }

                let first = results.first();
                match first {
                    Some(first) => {
                        let res = String::from_utf8_lossy(first).to_string();
                        info!("tilley result {res}");
                        if res != "100" {
                            return Ok(server::Auth::reject());
                        } else {
                            return Ok(server::Auth::Accept);
                        }
                    }
                    None => {
                        return Ok(server::Auth::reject());
                    }
                };
            }

            let prompts: Cow<'static, [(Cow<'static, str>, bool)]> =
                Cow::Borrowed(&[(Cow::Borrowed("Enter your gay level: "), true)]);
            info!("tilley detected");
            return Ok(server::Auth::Partial {
                name: Cow::Borrowed("Additional tests required"),
                instructions: Cow::Borrowed("enter your level of gayness (1-100)"),
                prompts,
            });
        }
        Ok(server::Auth::Accept)
    }

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        info!(user = ?user, password = ?password, "auth_password");
        Ok(server::Auth::Accept)
    }
}

fn move_left(state: &mut ClientState, output: &mut Vec<CryptoVec>) {
    if state.cursor > 0 {
        state.cursor -= 1;
        output.push(CryptoVec::from("\x1b[D"));
    }
}

fn move_right(state: &mut ClientState, output: &mut Vec<CryptoVec>) {
    if state.cursor < state.buffer.len() {
        state.cursor += 1;
        output.push(CryptoVec::from("\x1b[C"));
    }
}

fn redraw_line(state: &mut ClientState, output: &mut Vec<CryptoVec>) {
    output.push(CryptoVec::from("\r"));

    output.push(CryptoVec::from("\x1b[2K"));

    output.push(CryptoVec::from("> "));
    output.push(CryptoVec::from(state.buffer.clone()));

    let right_shift = state.buffer.len() - state.cursor;
    if right_shift > 0 {
        output.push(CryptoVec::from(format!("\x1b[{}D", right_shift)));
    }
}

type Sequence<'a> = &'a [(&'a [u8], fn(&mut ClientState, &mut Vec<CryptoVec>))];
fn handle_escape(state: &mut ClientState, output: &mut Vec<CryptoVec>) {
    const SEQUENCES: Sequence = &[
        // left
        (b"\x1b[D", move_left),
        // right
        (b"\x1b[C", move_right),
        // // delete
        (b"\x1b[3~", |s, out| {
            if s.cursor < s.buffer.len() {
                s.buffer.remove(s.cursor);
                redraw_line(s, out);
            }
        }),
    ];

    let escape_bytes = &state.escape;

    // Try to find a full match
    for (seq, action) in SEQUENCES {
        if escape_bytes == *seq {
            action(state, output);
            state.escape.clear();
            return;
        }
    }

    // Check if escape_bytes is a **prefix** of any valid sequence
    if !SEQUENCES
        .iter()
        .any(|(seq, _)| seq.starts_with(escape_bytes))
    {
        // Invalid / unrecognized sequence → clear buffer
        state.escape.clear();
    }
}

fn load_or_generate_key(path: &str) -> anyhow::Result<PrivateKey> {
    if Path::new(path).exists() {
        let data = fs::read(path)?;
        Ok(PrivateKey::from_openssh(&data)?)
    } else {
        let key = PrivateKey::random(&mut OsRng, russh::keys::Algorithm::Ed25519)?;
        fs::write(
            path,
            key.clone()
                .to_openssh(russh::keys::ssh_key::LineEnding::CRLF)?,
        )?;
        Ok(key)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let key_path = "./ssh_host_ed25519";
    let key = load_or_generate_key(key_path)?;
    let config = russh::server::Config {
        inactivity_timeout: Some(std::time::Duration::from_secs(600)),
        auth_rejection_time: std::time::Duration::from_secs(3),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        keys: vec![key],
        ..Default::default()
    };
    let config = Arc::new(config);

    let mut sh = Server {
        clients: Arc::new(Mutex::new(HashMap::new())),
        id: 0,
    };

    let socket = TcpListener::bind(("0.0.0.0", 2222)).await.unwrap();
    let server = sh.run_on_socket(config, &socket);

    server.await.unwrap();
    Ok(())
}
