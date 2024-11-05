pub struct Point {
    pub y: i32,
    pub x: i32,
}

impl Point {
    pub fn greater_than(&self, other: &Self) -> bool {
        if self.y > other.y { return true }
        self.y == other.y && self.x > other.x
    }
    pub fn less_than(&self, other: &Self) -> bool {
        if self.y < other.y { return true }
        self.y == other.y && self.x < other.x
    }
    pub fn greater_equal(&self, other: &Self) -> bool {
        if self.y > other.y {
            return true
        }
        if self.y == other.y && self.x >= other.x  {
            return true
        }
        return false
    }
    pub fn equal(&self, other: &Self) -> bool {
        self.y == other.y && self.x == other.x
    }

}


pub struct Selection {
    pub start: Point,
    pub end: Point,
    pub active: bool,
    pub keep_once: bool
}

impl Selection {
    pub fn new() ->  Self {
        Self {
            start: Point { y: -1, x: -1 },
            end: Point { y: -1, x: -1 },
            active: false,
            keep_once: false,
        }
    }
    pub fn clean(&mut self) {
        self.start.y = -1;
        self.start.x = -1;
        self.end.y = -1;
        self.end.x = -1;
        self.active = false;
    }

    pub fn activate(&mut self) {
        self.active = true;
    }

    pub fn empty(&mut self) -> bool {
        if self.start.x == -1 || self.start.y == -1 || self.end.x == -1 || self.end.y == -1 { return true }
        let equal = self.start.equal(&self.end);
        equal
    }
    pub fn non_empty(&mut self) -> bool {
        if self.start.x == -1 || self.start.y == -1 || self.end.x == -1 || self.end.y == -1 { return false }
        !self.start.equal(&self.end)
    }
    pub fn non_empty_and_active(&mut self) -> bool {
        self.non_empty() && (self.active || self.keep_once)
    }
    pub fn set_start(&mut self, y: usize, x: usize) {
        self.start.x = x as i32;
        self.start.y = y as i32;
    }
    pub fn set_end(&mut self, y: usize, x: usize) {
        self.end.x = x as i32;
        self.end.y = y as i32;
    }

    pub fn contains(&mut self, y: usize, x: usize) -> bool {
        if self.empty() { return false }

        let p = Point {x: x as i32, y: y as i32};

        let result = if self.start.greater_than(&self.end) {
            p.greater_equal(&self.end) && p.less_than(&self.start)
        } else {
            p.greater_equal(&self.start) && p.less_than(&self.end)
        };

        result
    }

    pub fn is_selected(&mut self, y: usize, x: usize) -> bool {
        let allowed = self.active || self.keep_once;
        let contains = self.contains(y, x);
        allowed && contains
    }

    pub fn from(&mut self) -> (usize, usize) {
        if self.start.greater_than(&self.end) { (self.end.y as usize, self.end.x as usize)  }
        else { (self.start.y as usize, self.start.x as usize) }
    }
    pub fn to(&mut self) -> (usize, usize) {
        if self.start.greater_than(&self.end) { (self.start.y as usize, self.start.x as usize)  }
        else { (self.end.y as usize, self.end.x as usize) }
    }

    pub fn swap(&mut self) {
        if self.start.greater_than(&self.end) {
            std::mem::swap(&mut self.start, &mut self.end);
        }
    }
}