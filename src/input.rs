use minifb::{Key, Window};

pub struct Input<'a> {
    win: &'a Window,
}

impl<'a> Input<'a> {
    pub fn is_key_down(&self, key: Key) -> bool {
        self.win.is_key_down(key)
    }
}
