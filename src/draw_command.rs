#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }
}

// PICO-8 Color Palette
// Source: https://pico-8.fandom.com/wiki/Palette

pub const BLACK: Color = Color::from_rgb(0, 0, 0);
pub const DARK_BLUE: Color = Color::from_rgb(29, 43, 83);
pub const DARK_PURPLE: Color = Color::from_rgb(126, 37, 83);
pub const DARK_GREEN: Color = Color::from_rgb(0, 135, 81);
pub const BROWN: Color = Color::from_rgb(171, 82, 54);
pub const DARK_GREY: Color = Color::from_rgb(95, 87, 79);
pub const LIGHT_GREY: Color = Color::from_rgb(194, 195, 199);
pub const WHITE: Color = Color::from_rgb(255, 241, 232);
pub const RED: Color = Color::from_rgb(255, 0, 77);
pub const ORANGE: Color = Color::from_rgb(255, 163, 0);
pub const YELLOW: Color = Color::from_rgb(255, 236, 39);
pub const GREEN: Color = Color::from_rgb(0, 228, 54);
pub const BLUE: Color = Color::from_rgb(41, 173, 255);
pub const LAVENDER: Color = Color::from_rgb(131, 118, 156);
pub const PINK: Color = Color::from_rgb(255, 119, 168);
pub const LIGHT_PEACH: Color = Color::from_rgb(255, 204, 170);

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
