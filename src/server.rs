use crate::Game;
use crate::game::Key;
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, accept};

const INDEX_HTML: &str = include_str!("client.html");
const ADDR: &str = "0.0.0.0:3000";
const BUFFER_CAP: usize = 4;

pub fn run_game<T: Game>() {
    let listener = TcpListener::bind(ADDR).unwrap();
    println!("Listening on {ADDR} — open http://<vm-ip>:3000");

    let mut ws = loop {
        let (stream, _) = listener.accept().unwrap();
        if let Some(ws) = handle_connection::<T>(stream) {
            break ws;
        }
    };

    run_loop::<T>(&mut ws);
}

fn handle_connection<T: Game>(mut stream: TcpStream) -> Option<WebSocket<TcpStream>> {
    let mut buf = [0u8; 4096];
    let n = stream.peek(&mut buf).ok()?;
    let req = std::str::from_utf8(&buf[..n]).unwrap_or("");

    if req.to_lowercase().contains("upgrade: websocket") {
        accept(stream).ok()
    } else {
        let _ = stream.read(&mut buf);
        let body = INDEX_HTML.replace("__NAME__", T::NAME);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        None
    }
}

fn run_loop<T: Game>(ws: &mut WebSocket<TcpStream>) {
    let args: Vec<String> = env::args().collect();
    let (mut state, mut fb) = T::init(args);
    let frame_dur = Duration::from_millis(1000 / T::FPS as u64);
    let mut held: Vec<Key> = Vec::new();
    let mut prev_held: Vec<Key> = Vec::new();
    let mut buffer: std::collections::VecDeque<Key> =
        std::collections::VecDeque::with_capacity(BUFFER_CAP);
    let mut mouse: Option<(u8, u8)> = None;

    // Show the initial frame from `new` before the first tick of input.
    if ws.send(Message::Binary(fb.as_bytes().to_vec())).is_err() {
        return;
    }

    loop {
        let start = Instant::now();

        // Union of all key-states observed this frame, so a tap that lands
        // press+release in one drain still shows up to the game.
        let mut seen: Option<Vec<Key>> = None;

        ws.get_mut().set_nonblocking(true).ok();
        loop {
            match ws.read() {
                Ok(Message::Text(s)) => {
                    if let Some(rest) = s.strip_prefix("M:") {
                        mouse = parse_mouse(rest);
                    } else {
                        let new_keys = parse_keys(&s);
                        match &mut seen {
                            Some(acc) => {
                                for k in &new_keys {
                                    if !acc.contains(k) {
                                        acc.push(*k);
                                    }
                                }
                            }
                            None => seen = Some(new_keys.clone()),
                        }
                        held = new_keys;
                    }
                }
                Ok(Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(_) => return,
            }
        }
        ws.get_mut().set_nonblocking(false).ok();

        let effective_held = seen.unwrap_or_else(|| held.clone());
        // Queue rising-edge presses (keys held this frame that weren't last).
        for k in &effective_held {
            if !prev_held.contains(k) && buffer.len() < BUFFER_CAP {
                buffer.push_back(*k);
            }
        }
        let buffered = buffer.pop_front();
        (state, fb) = T::update(state, &effective_held, buffered, mouse);
        prev_held = held.clone();

        if ws.send(Message::Binary(fb.as_bytes().to_vec())).is_err() {
            return;
        }

        let elapsed = start.elapsed();
        if elapsed < frame_dur {
            std::thread::sleep(frame_dur - elapsed);
        }
    }
}

fn parse_keys(s: &str) -> Vec<Key> {
    s.split(',')
        .filter(|t| !t.is_empty())
        .filter_map(name_to_key)
        .collect()
}

/// "x,y" → Some((x, y)); empty string → None (cursor left the canvas).
fn parse_mouse(s: &str) -> Option<(u8, u8)> {
    let (xs, ys) = s.split_once(',')?;
    Some((xs.parse().ok()?, ys.parse().ok()?))
}

fn name_to_key(name: &str) -> Option<Key> {
    match name {
        "Up" => Some(Key::Up),
        "Right" => Some(Key::Right),
        "Down" => Some(Key::Down),
        "Left" => Some(Key::Left),
        "Z" => Some(Key::Z),
        "X" => Some(Key::X),
        _ => None,
    }
}
