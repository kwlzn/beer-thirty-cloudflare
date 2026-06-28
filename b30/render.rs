//
// Renders the rated, sorted tap list into the HTML table the worker returns.
// Replaces the previous Polars-DataFrame-based renderer; takes plain structs so
// it is pure and host-testable, and drops a heavy dependency.
//

use crate::model::RatedBeer;

const HEADERS: [&str; 9] = [
    "category", "tap", "brewery", "name", "abv", "origin", "style", "age", "rating",
];

const STYLE: &str = r#"
<head>
  <style>
    table {
        border-collapse: collapse;
        width: 100%;
        margin: 20px 0;
        font-family: Arial, sans-serif;
    }
    th, td {
        border: 1px solid #ddd;
        padding: 8px;
        text-align: left;
        vertical-align: middle;
    }
    th {
        background-color: #f2f2f2;
        font-weight: bold;
        text-align: center !important;  /* Force center alignment for all headers */
    }
    tr:nth-child(even) td:not(.category-cell) {
        background-color: #f9f9f9;
    }
    tr:nth-child(odd) td:not(.category-cell) {
        background-color: #ffffff;
    }
    tr:hover td:not(.category-cell) {
        background-color: #f5f5f5;
    }
    .category-cell {
        font-weight: bold;
        text-align: center;
    }
    .category-cell-even {
        background-color: #f0f6fc;
    }
    .category-cell-odd {
        background-color: #ffffff;
    }
    .center {
        text-align: center;
    }
    /* The tap number is the thing a patron needs to order — make it pop. */
    .tap-cell {
        text-align: center;
        font-weight: bold;
        font-size: 1.4em;
        color: #1971c2;
    }
    /* ABV and rating cells get a continuously-interpolated background color
       (Excel-style color scale) emitted inline per cell — see ColorScale. */
    .rating-na {
        background-color: #e0e0e0 !important;
        color: #888;
    }
    /* Review links inherit the cell's (black) text color. */
    td a {
        color: inherit;
    }
  </style>
</head>
<body>
"#;

type Rgb = (u8, u8, u8);

/// A continuous color scale (Excel-style): blends between ascending value→color
/// stops and clamps outside the range. Both heatmap columns share this; they
/// only differ in the stops they supply.
struct ColorScale {
    stops: &'static [(f64, Rgb)],
}

impl ColorScale {
    /// Interpolated color at `value`.
    fn color(&self, value: f64) -> Rgb {
        let (lo_v, lo_c) = self.stops[0];
        if value <= lo_v {
            return lo_c;
        }
        let (hi_v, hi_c) = self.stops[self.stops.len() - 1];
        if value >= hi_v {
            return hi_c;
        }
        for pair in self.stops.windows(2) {
            let (v0, c0) = pair[0];
            let (v1, c1) = pair[1];
            if value <= v1 {
                let t = (value - v0) / (v1 - v0);
                return (
                    lerp(c0.0, c1.0, t),
                    lerp(c0.1, c1.1, t),
                    lerp(c0.2, c1.2, t),
                );
            }
        }
        hi_c
    }

    /// Inline `background-color`/`color` style for a heatmapped cell. Text is
    /// always black; the stops below are bright/light enough to keep it legible.
    fn style(&self, value: f64) -> String {
        let (r, g, b) = self.color(value);
        format!("background-color:#{r:02x}{g:02x}{b:02x};color:black")
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8
}

/// Rating heatmap. Bright RdYlGn palette (black text reads on all of it), tuned
/// for Untappd's distribution so a ~3.95 already reads green ("mostly
/// drinkable"), not yellow.
const RATING_SCALE: ColorScale = ColorScale {
    stops: &[
        (3.0, (0xd7, 0x30, 0x27)),  // red
        (3.4, (0xf4, 0x6d, 0x43)),  // orange
        (3.6, (0xfd, 0xae, 0x61)),  // light orange
        (3.8, (0xfe, 0xe0, 0x8b)),  // yellow
        (3.95, (0xa6, 0xd9, 0x6a)), // light green
        (4.15, (0x66, 0xbd, 0x63)), // green
        (4.4, (0x1a, 0x98, 0x50)),  // dark green
    ],
};

/// ABV heatmap (monotonic). The green ramp is spread across the populated
/// 4.5-7% range so low vs mid strengths read as visibly different greens; 7% is
/// a deliberate jump to yellowish-green, then orange through ~10% and red above.
const ABV_SCALE: ColorScale = ColorScale {
    stops: &[
        (0.0, (0x1a, 0x98, 0x50)),  // dark green
        (4.5, (0x4c, 0xb0, 0x50)),  // green
        (5.5, (0x84, 0xc9, 0x5a)),  // medium-light green
        (6.5, (0xc4, 0xe0, 0x7c)),  // light green
        (7.0, (0xd9, 0xe8, 0x4c)),  // yellowish-green (noticeable jump at 7%)
        (7.5, (0xfd, 0xae, 0x61)),  // light orange
        (9.5, (0xf4, 0x6d, 0x43)),  // orange
        (10.0, (0xd7, 0x30, 0x27)), // red (10%+)
    ],
};

/// Extract the numeric score from a rendered rating cell (our own
/// `<a href=...>3.62</a>` format). Returns `None` for "N/A" / unrated cells.
fn rating_value(cell: &str) -> Option<f64> {
    let inner = cell.strip_suffix("</a>")?;
    inner.rsplit('>').next()?.trim().parse::<f64>().ok()
}

fn category_label(category: &str) -> &str {
    if category.trim().is_empty() {
        "(Uncategorized)"
    } else {
        category
    }
}

/// Render the (already sorted) rated beers into an HTML table. Consecutive rows
/// sharing a category are merged into a single rowspanned category cell.
pub fn render(beers: &[RatedBeer]) -> String {
    let mut html = String::from(STYLE);

    html.push_str("<table>\n<thead>\n<tr>");
    for header in HEADERS {
        html.push_str(&format!("<th>{header}</th>"));
    }
    html.push_str("</tr>\n</thead>\n<tbody>\n");

    let mut row = 0;
    let mut category_number = 0;
    while row < beers.len() {
        let category = category_label(&beers[row].entry.category);

        // How many consecutive rows share this category.
        let mut count = 1;
        while row + count < beers.len()
            && category_label(&beers[row + count].entry.category) == category
        {
            count += 1;
        }

        let category_class = if category_number % 2 == 0 {
            "category-cell category-cell-even"
        } else {
            "category-cell category-cell-odd"
        };
        let display = if category == "(Uncategorized)" {
            ""
        } else {
            category
        };

        for offset in 0..count {
            let b = &beers[row + offset];
            let e = &b.entry;
            html.push_str("<tr>");
            if offset == 0 {
                html.push_str(&format!(
                    "<td class=\"{category_class}\" rowspan=\"{count}\">{display}</td>"
                ));
            }
            html.push_str(&format!("<td class=\"tap-cell\">{}</td>", e.tap_number));
            html.push_str(&format!("<td>{}</td>", e.brewery));
            html.push_str(&format!("<td>{}</td>", e.name));
            html.push_str(&format!(
                "<td class=\"center\" style=\"{}\">{}</td>",
                ABV_SCALE.style(e.abv_value()),
                e.abv
            ));
            html.push_str(&format!("<td class=\"center\">{}</td>", e.origin));
            html.push_str(&format!("<td>{}</td>", e.style));
            html.push_str(&format!("<td class=\"center\">{}</td>", e.days_old));
            match rating_value(&b.rating_html) {
                Some(score) => html.push_str(&format!(
                    "<td class=\"center\" style=\"{}\">{}</td>",
                    RATING_SCALE.style(score),
                    b.rating_html
                )),
                None => html.push_str(&format!(
                    "<td class=\"rating-na center\">{}</td>",
                    b.rating_html
                )),
            }
            html.push_str("</tr>\n");
        }

        category_number += 1;
        row += count;
    }

    html.push_str("</tbody>\n</table>");
    html.push_str("</body>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{sort_rated, BeerEntry};

    fn beer(category: &str, abv: &str, name: &str, rating: &str) -> RatedBeer {
        RatedBeer {
            entry: BeerEntry {
                tap_number: 1,
                brewery: "Test Brewery".into(),
                name: name.into(),
                abv: abv.into(),
                category: category.into(),
                origin: "Somewhere".into(),
                style: "IPA".into(),
                days_old: 3,
            },
            rating_html: rating.into(),
        }
    }

    #[test]
    fn groups_categories_with_rowspan() {
        let mut beers = vec![
            beer("IPA", "6.0", "B", "N/A"),
            beer("IPA", "5.0", "A", "4.10"),
            beer("Sour", "4.0", "C", "3.50"),
        ];
        sort_rated(&mut beers);
        let html = render(&beers);

        // Two IPAs merged under one rowspanned category cell.
        assert!(html.contains("rowspan=\"2\""));
        assert!(html.contains("rowspan=\"1\""));
        // Numeric ABV sort: 5.0 before 6.0 within IPA.
        assert!(html.find(">A<").unwrap() < html.find(">B<").unwrap());
        // Heatmap background color emitted inline.
        assert!(html.contains("background-color:#"));
        // Tap number is emphasized.
        assert!(html.contains("class=\"tap-cell\""));
    }

    #[test]
    fn scale_blends_between_stops() {
        // Exact stop values reproduce the stop color.
        assert_eq!(RATING_SCALE.color(3.6), (0xfd, 0xae, 0x61));
        assert_eq!(RATING_SCALE.color(3.95), (0xa6, 0xd9, 0x6a));
        // Clamps outside the range.
        assert_eq!(RATING_SCALE.color(1.0), (0xd7, 0x30, 0x27));
        assert_eq!(RATING_SCALE.color(5.0), (0x1a, 0x98, 0x50));
        // The whole point: nearby scores get different shades.
        assert_ne!(RATING_SCALE.color(3.90), RATING_SCALE.color(3.95));
        // A midpoint is a true channelwise blend of its bracketing stops.
        let mid = RATING_SCALE.color(3.7); // halfway 3.6 -> 3.8
        assert!(mid.1 > 0xae && mid.1 < 0xe0); // green channel between the stops
    }

    #[test]
    fn abv_low_is_green_high_is_red() {
        let greenish = |v: f64| {
            let (r, g, _) = ABV_SCALE.color(v);
            i32::from(g) > i32::from(r)
        };
        // The low-mid range reads green.
        assert!(greenish(3.0));
        assert!(greenish(5.0));
        assert!(greenish(6.5));
        // 7% jumps toward yellow vs 6% (more red, less blue).
        let (r6, _, b6) = ABV_SCALE.color(6.0);
        let (r7, _, b7) = ABV_SCALE.color(7.0);
        assert!(r7 > r6 && b7 < b6);
        // The highest ABVs read red (red channel dominates), clamped above 10%.
        let (r, g, _) = ABV_SCALE.color(11.0);
        assert!(r > g);
        assert_eq!(ABV_SCALE.color(12.0), (0xd7, 0x30, 0x27));
    }

    #[test]
    fn abv_greens_are_spread_out() {
        // 4.9% and 6.2% should be visibly different greens, not near-identical.
        let a = ABV_SCALE.color(4.9);
        let b = ABV_SCALE.color(6.2);
        let dist = (i32::from(a.0) - i32::from(b.0)).abs()
            + (i32::from(a.1) - i32::from(b.1)).abs()
            + (i32::from(a.2) - i32::from(b.2)).abs();
        assert!(dist > 60, "expected noticeable color spread, got {dist}");
    }

    #[test]
    fn rating_3_95_reads_green() {
        // ~3.95 should be a (light) green, i.e. green channel dominates red.
        let (r, g, _b) = RATING_SCALE.color(3.95);
        assert!(g > r, "expected green to dominate at 3.95, got r={r} g={g}");
    }

    #[test]
    fn style_text_is_always_black() {
        // Every cell uses black text for a consistent look against the bright palette.
        assert!(RATING_SCALE.style(4.4).contains("color:black"));
        assert!(RATING_SCALE.style(3.95).contains("color:black"));
        assert!(ABV_SCALE.style(5.0).contains("color:black"));
        assert!(RATING_SCALE.style(4.4).starts_with("background-color:#"));
    }

    #[test]
    fn rating_value_parsing() {
        let cell = "<a href=\"https://untappd.com/b/x/1\">3.62</a>";
        assert_eq!(rating_value(cell), Some(3.62));
        assert_eq!(rating_value("N/A"), None);
    }
}
