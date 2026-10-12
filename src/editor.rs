use image::{Rgba, RgbaImage};

pub struct Editor {
    pub image: RgbaImage,
    undo: Vec<RgbaImage>,
    pub tool: String,
    pub color: Rgba<u8>,
    pub stroke_width: f32,
    start: Option<(i32, i32)>,
    previous: Option<(i32, i32)>,
}
impl Editor {
    pub fn new(image: RgbaImage) -> Self {
        Self {
            image,
            undo: Vec::new(),
            tool: "pen".into(),
            color: Rgba([232, 78, 55, 255]),
            stroke_width: 4.0,
            start: None,
            previous: None,
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn undo(&mut self) {
        if let Some(image) = self.undo.pop() {
            self.image = image;
        }
    }
    pub fn pointer(&mut self, event: &str, x: f32, y: f32) {
        let point = (
            x.round()
                .clamp(0.0, self.image.width().saturating_sub(1) as f32) as i32,
            y.round()
                .clamp(0.0, self.image.height().saturating_sub(1) as f32) as i32,
        );
        match event {
            "down" => {
                let undo_limit = (128 * 1024 * 1024 / self.image.as_raw().len().max(1)).clamp(1, 3);
                while self.undo.len() >= undo_limit {
                    self.undo.remove(0);
                }
                self.undo.push(self.image.clone());
                self.start = Some(point);
                self.previous = Some(point);
                if self.tool == "pen" {
                    self.line(point, point);
                }
            }
            "move" => {
                if self.start.is_some() {
                    if self.tool == "pen" {
                        self.line(self.previous.unwrap(), point);
                    }
                    // Slint draws the live rectangle/arrow/crop outline; commit on release.
                    self.previous = Some(point);
                }
            }
            "up" => {
                if let Some(start) = self.start.take() {
                    if self.tool == "crop" {
                        self.image = self.undo.last().unwrap().clone();
                        let (x, y, w, h) = bounds(start, point);
                        if w >= 2 && h >= 2 {
                            self.image =
                                image::imageops::crop_imm(&self.image, x, y, w, h).to_image();
                        }
                    } else if self.tool != "pen" {
                        self.image = self.undo.last().unwrap().clone();
                        self.shape(start, point);
                    }
                }
                self.previous = None;
            }
            "cancel" => {
                if self.start.take().is_some() {
                    self.undo();
                }
                self.previous = None;
            }
            _ => {}
        }
    }
    fn shape(&mut self, a: (i32, i32), b: (i32, i32)) {
        match self.tool.as_str() {
            "rect" | "crop" => {
                self.line(a, (b.0, a.1));
                self.line((b.0, a.1), b);
                self.line(b, (a.0, b.1));
                self.line((a.0, b.1), a);
            }
            "arrow" => {
                self.line(a, b);
                let angle = ((a.1 - b.1) as f32).atan2((a.0 - b.0) as f32);
                let length = ((a.0 - b.0) as f32).hypot((a.1 - b.1) as f32).min(32.0);
                for offset in [-0.55f32, 0.55] {
                    self.line(
                        b,
                        (
                            b.0 + (length * (angle + offset).cos()) as i32,
                            b.1 + (length * (angle + offset).sin()) as i32,
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    fn line(&mut self, a: (i32, i32), b: (i32, i32)) {
        let radius = (self.stroke_width / 2.0).round().clamp(1.0, 16.0) as i32;
        let (mut x, mut y) = a;
        let dx = (b.0 - x).abs();
        let sx = if x < b.0 { 1 } else { -1 };
        let dy = -(b.1 - y).abs();
        let sy = if y < b.1 { 1 } else { -1 };
        let mut error = dx + dy;
        loop {
            for yy in y - radius..=y + radius {
                for xx in x - radius..=x + radius {
                    if xx >= 0
                        && yy >= 0
                        && xx < self.image.width() as i32
                        && yy < self.image.height() as i32
                        && (xx - x).pow(2) + (yy - y).pow(2) <= radius.pow(2)
                    {
                        self.image.put_pixel(xx as u32, yy as u32, self.color);
                    }
                }
            }
            if (x, y) == b {
                break;
            }
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                x += sx;
            }
            if twice <= dx {
                error += dx;
                y += sy;
            }
        }
    }
}
fn bounds(a: (i32, i32), b: (i32, i32)) -> (u32, u32, u32, u32) {
    (
        a.0.min(b.0) as u32,
        a.1.min(b.1) as u32,
        (a.0 - b.0).unsigned_abs() + 1,
        (a.1 - b.1).unsigned_abs() + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strokes_crop_and_undo() {
        let mut e = Editor::new(RgbaImage::from_pixel(40, 30, Rgba([255, 255, 255, 255])));
        e.pointer("down", 2.0, 2.0);
        e.pointer("move", 20.0, 20.0);
        e.pointer("up", 20.0, 20.0);
        assert_eq!(e.image.get_pixel(10, 10).0, [232, 78, 55, 255]);
        e.undo();
        assert_eq!(e.image.get_pixel(10, 10).0, [255, 255, 255, 255]);
        e.tool = "crop".into();
        e.pointer("down", 5.0, 5.0);
        e.pointer("up", 14.0, 19.0);
        assert_eq!(e.image.dimensions(), (10, 15));
        e.undo();
        assert_eq!(e.image.dimensions(), (40, 30));
    }
}
