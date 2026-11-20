use iced::advanced::graphics::text::{cosmic_text::fontdb, font_system};
use std::{fmt::Write, sync::Once};

pub const RECOMMENDED: [&str; 4] = ["Fredoka", "Nunito", "Enz Pebble", "Rubik"];

pub fn load() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut system = font_system().write().expect("font catalog");
        for bytes in BUNDLED {
            system
                .load_font(std::borrow::Cow::Borrowed(bytes))
                .expect("bundled OFL font");
        }
    });
}

pub fn families() -> &'static [&'static str] {
    static FAMILIES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    load();
    FAMILIES.get_or_init(|| {
        let mut system = font_system().write().expect("font catalog");
        let mut names = std::collections::BTreeSet::new();
        for face in system.raw().db().faces() {
            for (name, _) in &face.families {
                if !name.trim().is_empty()
                    && name.len() <= 200
                    && !name.chars().any(char::is_control)
                {
                    names.insert(name.clone());
                }
            }
        }
        names
            .into_iter()
            .map(|name| Box::leak(name.into_boxed_str()) as &'static str)
            .collect()
    })
}

pub fn font(family: &str, weight: u16) -> iced::Font {
    load();
    let name = families()
        .iter()
        .copied()
        .find(|f| *f == family)
        .unwrap_or("Fredoka");
    iced::Font {
        family: iced::font::Family::Name(name),
        weight: match weight {
            0..=149 => iced::font::Weight::Thin,
            150..=249 => iced::font::Weight::ExtraLight,
            250..=349 => iced::font::Weight::Light,
            350..=449 => iced::font::Weight::Normal,
            450..=549 => iced::font::Weight::Medium,
            550..=649 => iced::font::Weight::Semibold,
            650..=749 => iced::font::Weight::Bold,
            750..=849 => iced::font::Weight::ExtraBold,
            _ => iced::font::Weight::Black,
        },
        ..Default::default()
    }
}

pub fn weights(family: &str) -> Vec<u16> {
    load();
    let mut system = font_system().write().expect("font catalog");
    let db = system.raw().db();
    let mut weights = Vec::new();
    let regular = db.faces().any(|face| {
        face.style == fontdb::Style::Normal && face.families.iter().any(|(n, _)| n == family)
    });
    for face in db.faces().filter(|face| {
        (!regular || face.style == fontdb::Style::Normal)
            && face.families.iter().any(|(n, _)| n == family)
    }) {
        weights.push(face.weight.0);
        db.with_face_data(face.id, |bytes, index| {
            if let Ok(face) = ttf_parser::Face::parse(bytes, index) {
                for axis in face.variation_axes() {
                    if axis.tag == ttf_parser::Tag::from_bytes(b"wght") {
                        weights.extend((100..=900).step_by(100).filter(|w| {
                            f32::from(*w) >= axis.min_value && f32::from(*w) <= axis.max_value
                        }));
                    }
                }
            }
        });
    }
    weights.sort_unstable();
    weights.dedup();
    weights
}

struct Outline(String);
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "M{x} {y}");
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "L{x} {y}");
    }
    fn quad_to(&mut self, x: f32, y: f32, x1: f32, y1: f32) {
        let _ = write!(self.0, "Q{x} {y} {x1} {y1}");
    }
    fn curve_to(&mut self, x: f32, y: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
        let _ = write!(self.0, "C{x} {y} {x1} {y1} {x2} {y2}");
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

// Numeric clocks deliberately use fixed digit cells. Their bounds cannot change
// with the minute or during animation, even when the selected face is proportional.
pub fn outlines(family: &str, weight: u16, time: &str) -> Option<(String, f32, f32)> {
    load();
    let mut system = font_system().write().ok()?;
    let db = system.raw().db();
    let id = db.query(&fontdb::Query {
        families: &[
            fontdb::Family::Name(family),
            fontdb::Family::Name("Fredoka"),
        ],
        weight: fontdb::Weight(weight),
        ..Default::default()
    })?;
    db.with_face_data(id, |bytes, index| {
        let mut face = ttf_parser::Face::parse(bytes, index).ok()?;
        face.set_variation(ttf_parser::Tag::from_bytes(b"wght"), f32::from(weight));
        let mut cell = 0.0_f32;
        let mut top = 0_i16;
        let mut bottom = 0_i16;
        for c in '0'..='9' {
            let g = face.glyph_index(c)?;
            cell = cell.max(f32::from(face.glyph_hor_advance(g)?));
            let b = face.glyph_bounding_box(g)?;
            top = top.max(b.y_max);
            bottom = bottom.min(b.y_min);
        }
        let scale = 1000.0 / f32::from(face.units_per_em());
        let line = (f32::from(top) - f32::from(bottom)) * scale;
        let lines: Vec<_> = time.lines().collect();
        let mut paths = String::new();
        let mut width = 0.0_f32;
        for (row, text) in lines.iter().enumerate() {
            let mut x = 16.0;
            for c in text.chars() {
                let glyph = face.glyph_index(c)?;
                let advance = f32::from(face.glyph_hor_advance(glyph)?);
                let step = if c.is_ascii_digit() { cell } else { advance };
                let mut outline = Outline(String::new());
                face.outline_glyph(glyph, &mut outline)?;
                let offset = x + (step - advance) * scale * 0.5;
                let y = 16.0 + f32::from(top) * scale + row as f32 * (line + 35.0);
                let _ = write!(
                    paths,
                    "<path transform='translate({offset} {y}) scale({scale} -{scale})' d='{}'/>",
                    outline.0
                );
                x += step * scale;
            }
            width = width.max(x + 16.0);
        }
        let height =
            line * lines.len() as f32 + 35.0 * (lines.len().saturating_sub(1)) as f32 + 32.0;
        if !width.is_finite()
            || !height.is_finite()
            || height <= 1.0
            || !(0.1..=30.0).contains(&(width / height))
        {
            return None;
        }
        Some((paths, width, height))
    })?
}

pub fn aspect(family: &str, weight: u16, stacked: bool) -> f32 {
    static CACHE: std::sync::Mutex<Option<(String, u16, bool, f32)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().expect("clock metrics");
    if let Some((name, w, stack, aspect)) = &*cache
        && name == family
        && *w == weight
        && *stack == stacked
    {
        return *aspect;
    }
    let time = if stacked { "00\n00" } else { "00:00" };
    let aspect = outlines(family, weight, time)
        .or_else(|| outlines("Fredoka", 600, time))
        .map_or(1.0, |(_, w, h)| w / h);
    *cache = Some((family.into(), weight, stacked, aspect));
    aspect
}

const BUNDLED: &[&[u8]] = &[
    include_bytes!("../assets/lock-fonts/Fredoka-300.ttf"),
    include_bytes!("../assets/lock-fonts/Fredoka-400.ttf"),
    include_bytes!("../assets/lock-fonts/Fredoka-500.ttf"),
    include_bytes!("../assets/lock-fonts/Fredoka-600.ttf"),
    include_bytes!("../assets/lock-fonts/Fredoka-700.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-200.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-300.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-400.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-500.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-600.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-700.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-800.ttf"),
    include_bytes!("../assets/lock-fonts/Nunito-900.ttf"),
    include_bytes!("../assets/lock-fonts/Quicksand-300.ttf"),
    include_bytes!("../assets/lock-fonts/Quicksand-400.ttf"),
    include_bytes!("../assets/lock-fonts/Quicksand-500.ttf"),
    include_bytes!("../assets/lock-fonts/Quicksand-600.ttf"),
    include_bytes!("../assets/lock-fonts/Quicksand-700.ttf"),
    include_bytes!("../assets/lock-fonts/Rubik-300.ttf"),
    include_bytes!("../assets/lock-fonts/Rubik-400.ttf"),
    include_bytes!("../assets/lock-fonts/Rubik-500.ttf"),
    include_bytes!("../assets/lock-fonts/Rubik-600.ttf"),
    include_bytes!("../assets/lock-fonts/Rubik-700.ttf"),
    include_bytes!("../assets/lock-fonts/Rubik-800.ttf"),
    include_bytes!("../assets/lock-fonts/Rubik-900.ttf"),
];

pub fn monogram(name: &str) -> String {
    load();
    let words: Vec<_> = name.split_whitespace().collect();
    let letters: String = words
        .first()
        .and_then(|s| s.chars().next())
        .into_iter()
        .chain(
            (words.len() > 1)
                .then(|| words.last().and_then(|s| s.chars().next()))
                .flatten(),
        )
        .flat_map(char::to_uppercase)
        .take(2)
        .collect();
    let mut system = font_system().write().expect("font catalog");
    let db = system.raw().db();
    let face = db.query(&fontdb::Query {
        families: &[fontdb::Family::Name("Nunito")],
        weight: fontdb::Weight(600),
        ..Default::default()
    });
    let path=face.and_then(|id|db.with_face_data(id,|bytes,index| {
        let face=ttf_parser::Face::parse(bytes,index).ok()?;
        let (mut x,mut minx,mut miny,mut maxx,mut maxy)=(0.0_f32,f32::MAX,f32::MAX,f32::MIN,f32::MIN);
        let mut paths=String::new();
        for c in letters.chars() {
            let g=face.glyph_index(c)?;let b=face.glyph_bounding_box(g)?;
            minx=minx.min(x+f32::from(b.x_min));maxx=maxx.max(x+f32::from(b.x_max));miny=miny.min(f32::from(b.y_min));maxy=maxy.max(f32::from(b.y_max));
            let mut o=Outline(String::new());face.outline_glyph(g,&mut o)?;
            let _=write!(paths,"<path transform='translate({x} 0)' d='{}'/>",o.0);
            x+=f32::from(face.glyph_hor_advance(g)?);
        }
        if paths.is_empty(){return None;}
        let scale=(54.0/(maxx-minx)).min(42.0/(maxy-miny));let tx=48.0-(minx+maxx)*scale*0.5;let ty=48.0+(miny+maxy)*scale*0.5;
        Some(format!("<g transform='translate({tx} {ty}) scale({scale} -{scale})' fill='#ffffff'>{paths}</g>"))
    })).flatten().unwrap_or_else(||"<circle cx='48' cy='36' r='12' fill='white'/><path d='M25 70q0-30 23-30t23 30' fill='white'/>".into());
    let colors = [
        ("#435c87", "#73618e"),
        ("#456c6c", "#6d8c7c"),
        ("#786381", "#b08284"),
        ("#65709e", "#8c97b1"),
    ];
    let hash = name
        .bytes()
        .fold(0_usize, |h, c| h.wrapping_mul(31).wrapping_add(c as usize));
    let (a, b) = colors[hash % colors.len()];
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'><defs><linearGradient id='avatar' x2='1' y2='1'><stop stop-color='{a}'/><stop offset='1' stop-color='{b}'/></linearGradient></defs><circle cx='48' cy='48' r='48' fill='url(#avatar)'/>{path}</svg>"
    )
}
