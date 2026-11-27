use super::{Dict, owned};
use std::fmt::Write;
use zbus::zvariant::Value;

/// A CUPS queue and its supported print options.
#[derive(Clone, Debug)]
pub struct Printer {
    pub(super) name: String,
    pub(super) label: String,
    pub(super) is_default: bool,
    pub(super) papers: Vec<Paper>,
    pub(super) default_paper: usize,
    pub(super) duplex: Vec<Duplex>,
    pub(super) default_duplex: Option<Duplex>,
    pub(super) color: Vec<bool>,
    pub(super) default_color: Option<bool>,
    pub(super) max_copies: u32,
}

impl Printer {
    /// The CUPS queue name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The printer's description for display, or its queue name when it has none.
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Papers supported by the queue.
    pub fn papers(&self) -> &[Paper] {
        &self.papers
    }
    /// Sides supported by the queue.
    pub fn duplex(&self) -> &[Duplex] {
        &self.duplex
    }
    /// Whether monochrome and color are selectable.
    pub fn color(&self) -> &[bool] {
        &self.color
    }
    /// Largest supported copy count.
    pub fn max_copies(&self) -> u32 {
        self.max_copies
    }
}

/// A physical paper size with the printer's margins.
#[derive(Clone, Debug, PartialEq)]
pub struct Paper {
    pub(super) media: String,
    pub(super) label: String,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) margins: [f64; 4],
}

impl Paper {
    /// Printer-specific display name for the paper size.
    pub fn label(&self) -> &str {
        &self.label
    }

    fn name(&self) -> &str {
        self.media
            .rsplit_once('_')
            .filter(|(_, suffix)| {
                suffix.as_bytes().first().is_some_and(u8::is_ascii_digit)
                    && suffix.contains('x')
                    && (suffix.ends_with("in") || suffix.ends_with("mm"))
            })
            .map_or(&self.media, |(name, _)| name)
    }
}

impl Default for Paper {
    fn default() -> Self {
        Self {
            media: "iso_a4_210x297mm".into(),
            label: "A4".into(),
            width: 210.0,
            height: 297.0,
            margins: [0.0; 4],
        }
    }
}

macro_rules! print_choices {
    ($(#[$attr:meta])* $name:ident, $default:ident, $($variant:ident => $portal:literal, $ipp:literal);+ $(;)?) => {
        $(#[$attr])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $(#[doc = $portal] $variant),+
        }
        impl $name {
            /// All choices in display order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            fn portal(self) -> &'static str {
                match self { $(Self::$variant => $portal),+ }
            }
            pub(super) fn ipp(self) -> &'static str {
                match self { $(Self::$variant => $ipp),+ }
            }
        }
        impl Default for $name {
            fn default() -> Self { Self::$default }
        }
    }
}

print_choices! {
    /// Page orientation.
    Orientation, Portrait,
    Portrait => "portrait", "3";
    Landscape => "landscape", "4";
    ReversePortrait => "reverse_portrait", "6";
    ReverseLandscape => "reverse_landscape", "5";
}

print_choices! {
    /// Sides and binding edge of a print job.
    Duplex, OneSided,
    OneSided => "simplex", "one-sided";
    LongEdge => "vertical", "two-sided-long-edge";
    ShortEdge => "horizontal", "two-sided-short-edge";
}

/// Which pages to print.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pages {
    /// The entire document.
    #[default]
    All,
    /// The current page, rendered by the application.
    Current,
    /// The selection, rendered by the application.
    Selection,
    /// One-based page ranges specified by the user.
    Range,
}

impl Pages {
    fn portal(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Current => "current",
            Self::Selection => "selection",
            Self::Range => "ranges",
        }
    }
}

fn ranges(input: &str, source_base: u32, output_base: u32) -> Option<String> {
    let page = |text: &str| {
        text.trim()
            .parse::<u32>()
            .ok()?
            .checked_sub(source_base)?
            .checked_add(output_base)
    };
    let mut output = String::new();
    for part in input.split(',') {
        if !output.is_empty() {
            output.push(',');
        }
        if let Some((start, end)) = part.split_once('-') {
            let (start, end) = (page(start)?, page(end)?);
            if end < start {
                return None;
            }
            write!(output, "{start}-{end}").ok()?;
        } else {
            write!(output, "{}", page(part)?).ok()?;
        }
    }
    Some(output)
}

/// A selection constrained to the available printer options.
#[derive(Clone, Debug)]
pub struct Selection {
    /// Index into the request's printer list.
    pub printer_index: usize,
    printer: Printer,
    /// Number of copies, as entered in the dialog.
    pub copies: String,
    /// Selected page mode.
    pub pages: Pages,
    /// One-based page ranges, as entered in the dialog.
    pub page_ranges: String,
    /// Selected orientation.
    pub orientation: Orientation,
    /// Index of the selected paper.
    pub paper: usize,
    /// Selected binding mode.
    pub duplex: Option<Duplex>,
    /// Selected color mode, when reported by the printer.
    pub color: Option<bool>,
}

impl Selection {
    pub(super) fn from_hints(
        printers: &[Printer],
        settings: &Dict,
        setup: &Dict,
        current: bool,
        selected: bool,
    ) -> Option<Self> {
        let hint = |key| {
            settings
                .get(key)
                .and_then(|value| <&str>::try_from(value).ok())
        };
        let setup_hint = |key| {
            setup
                .get(key)
                .and_then(|value| <&str>::try_from(value).ok())
        };
        let index = hint("printer")
            .and_then(|name| printers.iter().position(|p| p.name == name))
            .or_else(|| printers.iter().position(|p| p.is_default))
            .unwrap_or(0);
        let printer = printers.get(index)?;
        let orientation = hint("orientation")
            .or_else(|| setup_hint("Orientation"))
            .unwrap_or_default()
            .replace('-', "_");
        let page_ranges = hint("page-ranges")
            .filter(|value| value.len() <= 256)
            .and_then(|value| ranges(value, 0, 1))
            .unwrap_or_default();
        let copy_count = hint("n-copies")
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(1);
        let paper = hint("paper-format").or_else(|| setup_hint("Name"));
        Some(Self {
            printer_index: index,
            printer: printer.clone(),
            copies: copy_count.clamp(1, printer.max_copies).to_string(),
            pages: match hint("print-pages") {
                Some("current") if current => Pages::Current,
                Some("selection") if selected => Pages::Selection,
                Some("ranges") if !page_ranges.is_empty() => Pages::Range,
                _ => Pages::All,
            },
            page_ranges,
            orientation: Orientation::ALL
                .iter()
                .copied()
                .find(|value| value.portal() == orientation)
                .unwrap_or_default(),
            paper: paper
                .and_then(|name| {
                    printer
                        .papers
                        .iter()
                        .position(|paper| paper.media == name || paper.name() == name)
                })
                .unwrap_or(printer.default_paper),
            duplex: printer
                .duplex
                .iter()
                .copied()
                .find(|d| Some(d.portal()) == hint("duplex"))
                .or(printer.default_duplex),
            color: hint("use-color")
                .and_then(|value| value.parse::<bool>().ok())
                .filter(|value| printer.color.contains(value))
                .or(printer.default_color),
        })
    }

    /// Change the destination and adopt its paper, duplex and color defaults.
    pub fn choose_printer(&mut self, index: usize, printer: &Printer) {
        self.printer_index = index;
        self.printer = printer.clone();
        self.paper = printer.default_paper;
        self.duplex = printer.default_duplex;
        self.color = printer.default_color;
        if let Ok(copies) = self.copies.parse::<u32>() {
            self.copies = copies.clamp(1, printer.max_copies).to_string();
        }
    }

    /// Whether the copy count is supported by the selected printer.
    pub fn valid_copies(&self) -> bool {
        self.copies
            .parse::<u32>()
            .is_ok_and(|copies| (1..=self.printer.max_copies).contains(&copies))
    }

    /// Whether the page range can be used for printing.
    pub fn valid_range(&self) -> bool {
        self.page_ranges.len() <= 256 && ranges(&self.page_ranges, 1, 1).is_some()
    }

    pub(super) fn queue(&self) -> &str {
        &self.printer.name
    }

    pub(super) fn cups_options(&self, prepared: bool) -> Vec<(&'static str, String)> {
        let mut options = vec![
            ("document-format", "application/pdf".into()),
            (
                "copies",
                if prepared {
                    "1".into()
                } else {
                    self.copies.clone()
                },
            ),
        ];
        if let Some(paper) = self.printer.papers.get(self.paper) {
            options.push(("media", paper.media.clone()));
        }
        if let Some(duplex) = self.duplex {
            options.push(("sides", duplex.ipp().into()));
        }
        if let Some(color) = self.color {
            options.push((
                "print-color-mode",
                if color { "color" } else { "monochrome" }.into(),
            ));
        }
        // A PreparePrint caller renders the selected pages, copies and orientation into the PDF.
        if !prepared {
            options.push(("orientation-requested", self.orientation.ipp().into()));
            if self.pages == Pages::Range
                && let Some(range) = ranges(&self.page_ranges, 1, 1)
            {
                options.push(("page-ranges", range));
            }
        }
        options
    }

    pub(super) fn vardicts(&self) -> (Dict, Dict) {
        let fallback = Paper::default();
        let paper = self.printer.papers.get(self.paper).unwrap_or(&fallback);
        let mut settings = Dict::new();
        let mut put = |key: &str, value: &str| {
            settings.insert(key.into(), owned(Value::from(value)));
        };
        put("printer", &self.printer.name);
        put("n-copies", &self.copies);
        put("orientation", self.orientation.portal());
        put("print-pages", self.pages.portal());
        put("paper-format", paper.name());
        put("paper-width", &paper.width.to_string());
        put("paper-height", &paper.height.to_string());
        if self.pages == Pages::Range
            && let Some(range) = ranges(&self.page_ranges, 1, 0)
        {
            put("page-ranges", &range);
        }
        if let Some(duplex) = self.duplex {
            put("duplex", duplex.portal());
        }
        if let Some(color) = self.color {
            put("use-color", if color { "true" } else { "false" });
        }
        let mut setup = Dict::from([
            ("Name".into(), owned(Value::from(paper.name()))),
            (
                "DisplayName".into(),
                owned(Value::from(paper.label.as_str())),
            ),
            ("Width".into(), paper.width.into()),
            ("Height".into(), paper.height.into()),
            (
                "Orientation".into(),
                owned(Value::from(self.orientation.portal().replace('_', "-"))),
            ),
        ]);
        for (key, margin) in ["MarginTop", "MarginBottom", "MarginLeft", "MarginRight"]
            .into_iter()
            .zip(paper.margins)
        {
            setup.insert(key.into(), margin.into());
        }
        (settings, setup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printer(name: &str, is_default: bool) -> Printer {
        Printer {
            name: name.into(),
            label: name.into(),
            is_default,
            papers: vec![Paper::default()],
            default_paper: 0,
            duplex: Duplex::ALL.to_vec(),
            default_duplex: Some(Duplex::OneSided),
            color: vec![false, true],
            default_color: Some(false),
            max_copies: 99,
        }
    }

    #[test]
    fn vardicts_and_cups_map_ranges_and_capabilities() {
        let hints = Dict::from_iter(
            [
                ("page-ranges", "0-2,4"),
                ("print-pages", "ranges"),
                ("duplex", "horizontal"),
                ("use-color", "true"),
                ("n-copies", "3"),
            ]
            .map(|(key, value)| (key.into(), owned(Value::from(value)))),
        );
        let setup = Dict::from([(
            "Orientation".into(),
            owned(Value::from("reverse-landscape")),
        )]);
        let choice = Selection::from_hints(
            &[printer("First", false), printer("Default", true)],
            &hints,
            &setup,
            false,
            false,
        )
        .unwrap();
        assert_eq!(choice.queue(), "Default");
        assert_eq!(choice.page_ranges, "1-3,5");
        let (returned, setup) = choice.vardicts();
        assert_eq!(<&str>::try_from(&returned["page-ranges"]).unwrap(), "0-2,4");
        assert_eq!(
            <&str>::try_from(&returned["paper-format"]).unwrap(),
            "iso_a4"
        );
        assert_eq!(f64::try_from(&setup["Width"]).unwrap(), 210.0);
        assert_eq!(
            <&str>::try_from(&setup["Orientation"]).unwrap(),
            "reverse-landscape"
        );
        let direct: std::collections::HashMap<_, _> =
            choice.cups_options(false).into_iter().collect();
        assert_eq!(direct["copies"], "3");
        assert_eq!(direct["sides"], "two-sided-short-edge");
        assert_eq!(direct["print-color-mode"], "color");
        assert_eq!(direct["page-ranges"], "1-3,5");
        assert_eq!(direct["orientation-requested"], "5");
        let prepared: std::collections::HashMap<_, _> =
            choice.cups_options(true).into_iter().collect();
        assert_eq!(prepared["copies"], "1");
        assert!(!prepared.contains_key("page-ranges"));
        assert!(!prepared.contains_key("orientation-requested"));
    }

    #[test]
    fn rejects_inverted_zero_empty_and_overflowing_ranges() {
        for invalid in ["3-2", "0,4", "2,", "1--3", "4294967296"] {
            assert_eq!(ranges(invalid, 1, 0), None, "{invalid}");
        }
        assert_eq!(ranges("4294967295", 0, 1), None);
    }
}
