//! Floating-window overview layout with a shared scale capped at native size.
//! Chooses the largest fitting row arrangement while preserving approximate spatial order.

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    fn center(self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
    /// Linear interpolation from `self` (`t` = 0) to `to` (`t` = 1).
    #[must_use]
    pub fn lerp(self, to: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self::new(
            mix(self.x, to.x),
            mix(self.y, to.y),
            mix(self.width, to.width),
            mix(self.height, to.height),
        )
    }
}

struct Plan {
    rows: Vec<Vec<usize>>,
    scale: f32,
}

/// The spread position of each of `windows`, in the same order, inside
/// `area`, with a scaled `gap` and an unscaled `caption` below each row.
pub fn layout(windows: &[Rect], area: Rect, gap: f32, caption: f32) -> Vec<Rect> {
    if windows.is_empty() || area.width < 1.0 || area.height < 1.0 {
        return windows.to_vec();
    }
    let gap = gap.max(0.0);
    let rects: Vec<Rect> = windows
        .iter()
        .map(|r| {
            Rect::new(
                finite(r.x),
                finite(r.y),
                r.width.max(1.0),
                r.height.max(1.0),
            )
        })
        .collect();
    let mut by_height: Vec<usize> = (0..rects.len()).collect();
    by_height.sort_by(|&a, &b| {
        rects[a]
            .center()
            .1
            .total_cmp(&rects[b].center().1)
            .then(rects[a].center().0.total_cmp(&rects[b].center().0))
    });
    let best = (1..=rects.len())
        .map(|count| plan(&rects, &by_height, count, area, gap, caption))
        // A row more must make the windows clearly larger.
        .reduce(|best, next| {
            if next.scale > best.scale * 1.02 {
                next
            } else {
                best
            }
        })
        .expect("at least one window");
    place(&rects, best, area, gap, caption)
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

// `count` rows filled in order of height on screen: a window starts a new
// row when more than half of it would pass the row's share of the width.
fn plan(rects: &[Rect], order: &[usize], count: usize, area: Rect, gap: f32, caption: f32) -> Plan {
    let total: f32 = rects.iter().map(|r| r.width).sum::<f32>()
        + gap * (rects.len() as f32 - count as f32).max(0.0);
    let share = total / count as f32;
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut width = 0.0;
    for &index in order {
        let w = rects[index].width;
        let more = rows.len() < count;
        let row = rows.last_mut().expect("a row");
        if !row.is_empty() && more && width + gap + w / 2.0 > share {
            rows.push(vec![index]);
            width = w;
        } else {
            if !row.is_empty() {
                width += gap;
            }
            row.push(index);
            width += w;
        }
    }
    for row in &mut rows {
        row.sort_by(|&a, &b| rects[a].center().0.total_cmp(&rects[b].center().0));
    }
    let (width, height) = extent(rects, &rows, gap);
    let picture_height = (area.height - rows.len() as f32 * caption).max(1.0);
    Plan {
        scale: (area.width / width).min(picture_height / height).min(1.0),
        rows,
    }
}

fn row_size(rects: &[Rect], row: &[usize], gap: f32) -> (f32, f32) {
    let width = row.iter().map(|&i| rects[i].width).sum::<f32>() + gap * (row.len() as f32 - 1.0);
    let height = row.iter().map(|&i| rects[i].height).fold(0.0, f32::max);
    (width, height)
}

fn extent(rects: &[Rect], rows: &[Vec<usize>], gap: f32) -> (f32, f32) {
    let sizes: Vec<(f32, f32)> = rows.iter().map(|row| row_size(rects, row, gap)).collect();
    let width = sizes.iter().map(|s| s.0).fold(0.0, f32::max);
    let height = sizes.iter().map(|s| s.1).sum::<f32>() + gap * (rows.len() as f32 - 1.0);
    (width, height)
}

fn place(rects: &[Rect], plan: Plan, area: Rect, gap: f32, caption: f32) -> Vec<Rect> {
    let scale = plan.scale;
    let (_, height) = extent(rects, &plan.rows, gap);
    let mut placed = rects.to_vec();
    let occupied_height = height * scale + plan.rows.len() as f32 * caption;
    let mut y = area.y + (area.height - occupied_height).max(0.0) / 2.0;
    for row in &plan.rows {
        let (row_width, row_height) = row_size(rects, row, gap);
        let width = row_width * scale;
        let was = row.iter().map(|&i| rects[i].center().0).sum::<f32>() / row.len() as f32;
        let centre = area.x + area.width / 2.0;
        let lean = centre + (was - centre) / 2.0;
        let mut x = (lean - width / 2.0).clamp(area.x, (area.x + area.width - width).max(area.x));
        for &index in row {
            let r = rects[index];
            let (w, h) = (r.width * scale, r.height * scale);
            placed[index] = Rect::new(x, y + (row_height * scale - h) / 2.0, w, h);
            x += w + gap * scale;
        }
        y += (row_height + gap) * scale + caption;
    }
    placed
}
