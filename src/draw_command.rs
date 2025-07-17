#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

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