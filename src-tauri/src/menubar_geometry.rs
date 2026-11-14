#[derive(Clone, Copy)]
pub(crate) struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub(crate) struct Placement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub arrow: f64,
}

pub(crate) fn place(tray: Bounds, work: Bounds, height: f64) -> Placement {
    let width = 400.0_f64.min((work.width - 16.0).max(1.0));
    let height = height.min((work.height - 16.0).max(1.0));
    let anchor = tray.x + tray.width / 2.0;
    let x = (anchor - (width - 136.0)).clamp(work.x + 8.0, work.x + work.width - width - 8.0);
    let y = (tray.y + tray.height + 2.0).clamp(work.y + 8.0, work.y + work.height - height - 8.0);
    Placement {
        x,
        y,
        width,
        height,
        arrow: (anchor - x - 16.0).clamp(24.0, (width - 56.0).max(24.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_right_edge_and_arrow_inside_the_selected_screen() {
        let work = Bounds {
            x: 0.0,
            y: 25.0,
            width: 1440.0,
            height: 875.0,
        };
        let p = place(
            Bounds {
                x: 1400.0,
                y: 0.0,
                width: 24.0,
                height: 24.0,
            },
            work,
            650.0,
        );
        assert_eq!(p.x + p.width, 1432.0);
        // At the extreme edge the pointer stays clear of the rounded corner.
        assert_eq!(p.arrow, 344.0);
        assert!(p.y + p.height <= 900.0);
    }

    #[test]
    fn supports_negative_screen_origins_and_short_work_areas() {
        let work = Bounds {
            x: -1920.0,
            y: -100.0,
            width: 1920.0,
            height: 500.0,
        };
        let p = place(
            Bounds {
                x: -600.0,
                y: -125.0,
                width: 24.0,
                height: 24.0,
            },
            work,
            900.0,
        );
        assert_eq!(p.height, 484.0);
        assert_eq!(p.y, -92.0);
        assert!(p.x >= work.x && p.x + p.width <= 0.0);
    }
}
