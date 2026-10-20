#![allow(dead_code)]

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

/// Fractional blocks for sub-cell precision
const FRACTIONAL_BLOCKS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

#[derive(Debug, Clone)]
pub struct GaugeValue {
    pub ratio: f64,
    pub color: Color,
}

impl GaugeValue {
    pub fn new(ratio: f64, color: Color) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            color,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MultiValueGauge {
    values: Vec<GaugeValue>,
    bg_color: Color,
    use_unicode: bool,
}

impl MultiValueGauge {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            bg_color: Color::Reset,
            use_unicode: false,
        }
    }

    pub fn add_value(mut self, ratio: f64, color: Color) -> Self {
        self.values.push(GaugeValue::new(ratio, color));
        self
    }

    pub fn background_color(mut self, color: Color) -> Self {
        self.bg_color = color;
        self
    }

    /// Enables high-precision sub-cell rendering using Unicode block characters
    pub fn use_unicode(mut self, enable: bool) -> Self {
        self.use_unicode = enable;
        self
    }
}

impl Widget for MultiValueGauge {
    fn render(mut self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        // 1. Sort values DESCENDING (largest ratio first, smallest on top)
        self.values.sort_by(|a, b| b.ratio.partial_cmp(&a.ratio).unwrap_or(std::cmp::Ordering::Equal));

        // 2. Initialize the canvas area with the background color
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_style(Style::default().bg(self.bg_color).fg(self.bg_color));
                }
            }
        }

        // 3. Render each bar from background (largest) to foreground (smallest)
        for val in &self.values {
            let total_width_exact = area.width as f64 * val.ratio;
            let full_cells = total_width_exact.floor() as u16;
            let remainder = total_width_exact - full_cells as f64;

            for y in area.top()..area.bottom() {
                // Draw the full solid block segments
                for x in 0..full_cells {
                    let cell_x = area.left() + x;
                    if cell_x < area.right() {
                        if let Some(cell) = buf.cell_mut((cell_x, y)) {
                            cell.set_char('█');
                            // The cell inherits the active foreground color
                            cell.set_style(Style::default().fg(val.color).bg(cell.bg));
                        }
                    }
                }

                // Handle the fractional edge block if Unicode flag is on
                if self.use_unicode && remainder > 0.0 && full_cells < area.width {
                    let cell_x = area.left() + full_cells;
                    if cell_x < area.right() {
                        if let Some(cell) = buf.cell_mut((cell_x, y)) {
                            // Map remainder (0.0 - 1.0) to an index (0 to 7)
                            let block_idx = ((remainder * 8.0).floor() as usize).min(7);
                            let block_char = FRACTIONAL_BLOCKS[block_idx];

                            // Dynamic Gradient Transition: 
                            // Set foreground to our color, background to whatever was beneath us
                            let underlying_fg = cell.fg; 
                            cell.set_char(block_char);
                            cell.set_style(Style::default().fg(val.color).bg(underlying_fg));
                        }
                    }
                }
            }
        }
    }
}
