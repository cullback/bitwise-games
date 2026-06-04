use crate::Game;
use minifb::Key;
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, accept};

const INDEX_HTML: &str = include_str!("client.html");
const ADDR: &str = "0.0.0.0:3000";

pub fn run_game<T: Game>() {
    let listener = TcpListener::bind(ADDR).unwrap();
    println!("Listening on {ADDR} — open http://<vm-ip>:9001");

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
        let body = INDEX_HTML
            .replace("__WIDTH__", &T::WIDTH.to_string())
            .replace("__HEIGHT__", &T::HEIGHT.to_string())
            .replace("__NAME__", T::NAME);
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
    let (mut state, mut fb) = T::new(args);
    let frame_dur = Duration::from_millis(1000 / T::FPS as u64);
    let mut held: Vec<Key> = Vec::new();
    let mut prev_held: Vec<Key> = Vec::new();

    loop {
        let start = Instant::now();

        // Union of all key-states observed this frame, so a tap that lands
        // press+release in one drain still shows up to the game.
        let mut seen: Option<Vec<Key>> = None;

        ws.get_mut().set_nonblocking(true).ok();
        loop {
            match ws.read() {
                Ok(Message::Text(s)) => {
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
        let pressed: Vec<Key> = effective_held
            .iter()
            .filter(|k| !prev_held.contains(k))
            .copied()
            .collect();
        (state, fb) = T::update(state, &effective_held, &pressed);
        prev_held = held.clone();

        let mut bytes = Vec::with_capacity(fb.len() * 4);
        for px in &fb {
            bytes.push(((px >> 16) & 0xff) as u8);
            bytes.push(((px >> 8) & 0xff) as u8);
            bytes.push((px & 0xff) as u8);
            bytes.push(0xff);
        }
        if ws.send(Message::Binary(bytes)).is_err() {
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

fn name_to_key(name: &str) -> Option<Key> {
    match name {
        "Left" => Some(Key::Left),
        "Right" => Some(Key::Right),
        "Up" => Some(Key::Up),
        "Down" => Some(Key::Down),
        "Space" => Some(Key::Space),
        "Enter" => Some(Key::Enter),
        "Escape" => Some(Key::Escape),
        "LeftShift" => Some(Key::LeftShift),
        "RightShift" => Some(Key::RightShift),
        "A" => Some(Key::A),
        "B" => Some(Key::B),
        "C" => Some(Key::C),
        "D" => Some(Key::D),
        "E" => Some(Key::E),
        "F" => Some(Key::F),
        "G" => Some(Key::G),
        "H" => Some(Key::H),
        "I" => Some(Key::I),
        "J" => Some(Key::J),
        "K" => Some(Key::K),
        "L" => Some(Key::L),
        "M" => Some(Key::M),
        "N" => Some(Key::N),
        "O" => Some(Key::O),
        "P" => Some(Key::P),
        "Q" => Some(Key::Q),
        "R" => Some(Key::R),
        "S" => Some(Key::S),
        "T" => Some(Key::T),
        "U" => Some(Key::U),
        "V" => Some(Key::V),
        "W" => Some(Key::W),
        "X" => Some(Key::X),
        "Y" => Some(Key::Y),
        "Z" => Some(Key::Z),
        _ => None,
    }
}
