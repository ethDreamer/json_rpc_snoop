use lazy_static::lazy_static;
use termion::color;

lazy_static! {
    pub static ref RESET_NEWLINE: String = format!("{}\n", color::Fg(color::Reset));
    pub static ref CYAN: String = color::Fg(color::Cyan).to_string();
    pub static ref RED: String = color::Fg(color::Red).to_string();
    pub static ref GREEN: String = color::Fg(color::Green).to_string();
    pub static ref EMPTY: String = String::new();
}

pub struct Colors {
    pub red: &'static str,
    pub cyan: &'static str,
    pub green: &'static str,
}

impl Colors {
    pub fn new(disable_color: bool) -> Self {
        if disable_color {
            Self {
                red: (*EMPTY).as_str(),
                cyan: (*EMPTY).as_str(),
                green: (*EMPTY).as_str(),
            }
        } else {
            Self {
                red: (*RED).as_str(),
                cyan: (*CYAN).as_str(),
                green: (*GREEN).as_str(),
            }
        }
    }
}

// Accepts a multi-line string and adds the color code in `color`.
// Each line will start with the color-code and reset the color before
// the start of the next line. This displays much better in utils like
// less than only setting the color at the beginning and end of a multi-
// line string.
pub fn color_treat(multi_line_string: String, color: &str) -> String {
    if color.is_empty() {
        return multi_line_string;
    }
    let mut result = String::new();
    for line in multi_line_string.split("\n") {
        result.push_str(&color);
        result.push_str(line);
        result.push_str((*RESET_NEWLINE).as_str());
    }

    result
}
