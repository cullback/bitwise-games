// PICO-8 Color Palette
// Source: https://pico-8.fandom.com/wiki/Palette
//
// The discriminants are the wire format: a FrameBuffer is sent to the client
// as raw palette indices, and the client maps each index back to RGB via the
// same table baked into client.html.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    DarkBlue = 1,
    DarkPurple = 2,
    DarkGreen = 3,
    Brown = 4,
    DarkGrey = 5,
    LightGrey = 6,
    White = 7,
    Red = 8,
    Orange = 9,
    Yellow = 10,
    Green = 11,
    Blue = 12,
    Lavender = 13,
    Pink = 14,
    LightPeach = 15,
}

pub const BLACK: Color = Color::Black;
pub const DARK_BLUE: Color = Color::DarkBlue;
pub const DARK_PURPLE: Color = Color::DarkPurple;
pub const DARK_GREEN: Color = Color::DarkGreen;
pub const BROWN: Color = Color::Brown;
pub const DARK_GREY: Color = Color::DarkGrey;
pub const LIGHT_GREY: Color = Color::LightGrey;
pub const WHITE: Color = Color::White;
pub const RED: Color = Color::Red;
pub const ORANGE: Color = Color::Orange;
pub const YELLOW: Color = Color::Yellow;
pub const GREEN: Color = Color::Green;
pub const BLUE: Color = Color::Blue;
pub const LAVENDER: Color = Color::Lavender;
pub const PINK: Color = Color::Pink;
pub const LIGHT_PEACH: Color = Color::LightPeach;

#[derive(Clone, Copy, Debug)]
pub struct Rectangle {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug)]
pub struct Line {
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug)]
pub struct Circle {
    pub x: u32,
    pub y: u32,
    pub radius: u32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug)]
pub enum DrawCommand {
    Rectangle(Rectangle),
    Line(Line),
    Circle(Circle),
}

impl DrawCommand {
    pub fn rect(x: u32, y: u32, width: u32, height: u32, color: Color) -> Self {
        Self::Rectangle(Rectangle {
            x,
            y,
            width,
            height,
            color,
        })
    }

    pub fn line(x1: u32, y1: u32, x2: u32, y2: u32, color: Color) -> Self {
        Self::Line(Line {
            x1,
            y1,
            x2,
            y2,
            color,
        })
    }

    pub fn circle(x: u32, y: u32, radius: u32, color: Color) -> Self {
        Self::Circle(Circle {
            x,
            y,
            radius,
            color,
        })
    }
}
