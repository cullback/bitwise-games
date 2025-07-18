use crate::draw_command::{Circle, Color, DrawCommand, Line, Rectangle};

pub struct FrameBuffer {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
}

impl FrameBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixels: vec![0; (width * height) as usize],
            width,
            height,
        }
    }

    pub fn draw(&mut self, command: &DrawCommand) {
        match command {
            DrawCommand::Rectangle(rect) => self.draw_rectangle(rect),
            DrawCommand::Line(line) => self.draw_line(line),
            DrawCommand::Circle(circle) => self.draw_circle(circle),
        }
    }

    fn draw_rectangle(&mut self, rect: &Rectangle) {
        for y in rect.y..(rect.y + rect.height) {
            for x in rect.x..(rect.x + rect.width) {
                self.set_pixel(x, y, &rect.color);
            }
        }
    }

    fn draw_line(&mut self, line: &Line) {
        // Bresenham's line algorithm
        let mut x0 = line.x1 as i32;
        let mut y0 = line.y1 as i32;
        let x1 = line.x2 as i32;
        let y1 = line.y2 as i32;
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            self.set_pixel(x0 as u32, y0 as u32, &line.color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn draw_circle(&mut self, circle: &Circle) {
        // Midpoint circle algorithm
        let x_center = circle.x as i32;
        let y_center = circle.y as i32;
        let mut x = circle.radius as i32;
        let mut y = 0;
        let mut err = 0;

        while x >= y {
            self.set_pixel((x_center + x) as u32, (y_center + y) as u32, &circle.color);
            self.set_pixel((x_center + y) as u32, (y_center + x) as u32, &circle.color);
            self.set_pixel((x_center - y) as u32, (y_center + x) as u32, &circle.color);
            self.set_pixel((x_center - x) as u32, (y_center + y) as u32, &circle.color);
            self.set_pixel((x_center - x) as u32, (y_center - y) as u32, &circle.color);
            self.set_pixel((x_center - y) as u32, (y_center - x) as u32, &circle.color);
            self.set_pixel((x_center + y) as u32, (y_center - x) as u32, &circle.color);
            self.set_pixel((x_center + x) as u32, (y_center - y) as u32, &circle.color);

            if err <= 0 {
                y += 1;
                err += 2 * y + 1;
            }
            if err > 0 {
                x -= 1;
                err -= 2 * x + 1;
            }
        }
    }

    fn set_pixel(&mut self, x: u32, y: u32, color: &Color) {
        if x < self.width && y < self.height {
            let index = (y * self.width + x) as usize;
            if index < self.pixels.len() {
                self.pixels[index] = ((color.a as u32) << 24)
                    | ((color.r as u32) << 16)
                    | ((color.g as u32) << 8)
                    | (color.b as u32);
            }
        }
    }
}
