use crossterm::{
    cursor::MoveTo,
    style::{Color, PrintStyledContent, Stylize},
    QueueableCommand,
};
use std::io::{stdout, Write};
use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct Cell {
    character: char,
    fg_color: Color,
    bg_color: Color,
}

impl Cell {
    pub fn new(character: char, fg_color: Color, bg_color: Color) -> Self {
        Self { character, fg_color, bg_color }
    }
}

impl fmt::Debug for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Custom formatting for Cell struct
        write!(f, 
            // "Cell {{ '{}' {:?} {:?} }}", 
            " {} ", 
            self.character, 
            // self.fg_color, 
            // self.bg_color
        )
    }
}

pub struct ScreenBuffer {
    width: usize,
    height: usize,
    cells: Vec<Vec<Option<Cell>>>,
}

impl ScreenBuffer {
    pub fn new(width: usize, height: usize) -> Self {
        let cells = vec![vec![None; width]; height];
        Self { width, height, cells }
    }

    pub fn set_cell(&mut self, x: usize, y: usize, cell: Cell) {
        if y < self.height && x < self.width {
            self.cells[y][x] = Some(cell);
        }
    }

    pub fn get_cell(&self, x: usize, y: usize) -> Option<&Cell> {
        if y < self.height && x < self.width {
            self.cells[y][x].as_ref()
        } else {
            None
        }
    }

    pub fn resize(&mut self, new_width: usize, new_height: usize) {
        let default_cell = Cell {
            character: ' ',
            fg_color: Color::Reset,
            bg_color: Color::Reset,
        };

        // Resize each row to match new_width
        for row in &mut self.cells {
            if new_width > self.width {
                row.extend(std::iter::repeat(Some(default_cell.clone())).take(new_width - self.width));
            } else {
                row.truncate(new_width);
            }
        }

        // Resize the outer vector to match new_height
        if new_height > self.height {
            let new_row = vec![Some(default_cell); new_width];
            self.cells
                .extend(std::iter::repeat(new_row).take(new_height - self.height));
        } else {
            self.cells.truncate(new_height);
        }

        self.width = new_width;
        self.height = new_height;
    }

    pub fn cell_equal(&self, x: usize, y: usize, other: &Cell) -> bool {
        if x >= self.width || y >= self.height {
            return false; // Out of bounds
        }
        match &self.cells[y][x] {
            Some(cell) => cell == other,
            None => false,
        }
    }

}

impl fmt::Debug for ScreenBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScreenBuffer {{ width: {}, height: {}, cells: [\n", self.width, self.height)?;

        for row in &self.cells {
            write!(f, "    [")?;
            for cell in row {
                match cell {
                    Some(c) => write!(f, "{:?}, ", c)?,
                    None => write!(f, "None, ")?,
                }
            }
            write!(f, "],\n")?;
        }

        write!(f, "] }}") // Close the `ScreenBuffer` output
    }
}
