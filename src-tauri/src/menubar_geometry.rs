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

#[cfg(any(target_os = "macos", test))]
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

/// Coordinates are logical pixels at the tray monitor's scale. The caller
/// converts the result back to physical pixels before moving a Windows window.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn place_windows(tray: Bounds, screen: Bounds, work: Bounds, height: f64) -> Placement {
    let gap = 8.0;
    let width = 400.0_f64.min((work.width - gap * 2.0).max(1.0));
    let height = height.min((work.height - gap * 2.0).max(1.0));
    let right = work.x + work.width;
    let bottom = work.y + work.height;
    let (x, y) = if work.y > screen.y + 1.0 {
        // Taskbar at the top.
        (right - width - gap, work.y.max(tray.y + tray.height) + gap)
    } else if work.x > screen.x + 1.0 {
        // Taskbar at the left.
        (
            work.x.max(tray.x + tray.width) + gap,
            tray.y + tray.height - height,
        )
    } else if right < screen.x + screen.width - 1.0 {
        // Taskbar at the right.
        (
            right.min(tray.x) - width - gap,
            tray.y + tray.height - height,
        )
    } else {
        // Bottom/autohidden taskbar. Overflow icons can be above the work-area
        // edge, so leave their actual clicked location unobstructed as well.
        (right - width - gap, bottom.min(tray.y) - height - gap)
    };
    Placement {
        x: x.clamp(work.x + gap, (right - width - gap).max(work.x + gap)),
        y: y.clamp(work.y + gap, (bottom - height - gap).max(work.y + gap)),
        width,
        height,
        arrow: 0.0,
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

    #[test]
    fn windows_bottom_taskbar_and_overflow_keep_the_panel_above_the_icon() {
        let screen = Bounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let work = Bounds {
            height: 1032.0,
            ..screen
        };
        for icon_y in [1032.0, 948.0] {
            let tray = Bounds {
                x: 1664.0,
                y: icon_y,
                width: 24.0,
                height: 24.0,
            };
            let p = place_windows(tray, screen, work, 560.0);
            assert_eq!((p.x, p.width), (1512.0, 400.0));
            assert_eq!(p.y + p.height, icon_y - 8.0);
        }
    }

    #[test]
    fn windows_supports_top_left_and_right_taskbars() {
        let screen = Bounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let cases = [
            (
                Bounds {
                    y: 48.0,
                    height: 1032.0,
                    ..screen
                },
                Bounds {
                    x: 1664.0,
                    y: 0.0,
                    width: 24.0,
                    height: 48.0,
                },
                (1512.0, 56.0),
            ),
            (
                Bounds {
                    x: 48.0,
                    width: 1872.0,
                    ..screen
                },
                Bounds {
                    x: 0.0,
                    y: 1000.0,
                    width: 48.0,
                    height: 24.0,
                },
                (56.0, 464.0),
            ),
            (
                Bounds {
                    width: 1872.0,
                    ..screen
                },
                Bounds {
                    x: 1872.0,
                    y: 1000.0,
                    width: 48.0,
                    height: 24.0,
                },
                (1464.0, 464.0),
            ),
        ];
        for (work, tray, expected) in cases {
            let p = place_windows(tray, screen, work, 560.0);
            assert_eq!((p.x, p.y), expected);
        }
    }

    #[test]
    fn windows_dpi_conversion_preserves_negative_monitor_coordinates_and_work_area() {
        // A 150% secondary display to the left of a 100% primary display.
        let scale = 1.5;
        let screen = Bounds {
            x: -2560.0 / scale,
            y: -200.0 / scale,
            width: 2560.0 / scale,
            height: 1440.0 / scale,
        };
        let work = Bounds {
            height: 1380.0 / scale,
            ..screen
        };
        let tray = Bounds {
            x: -180.0 / scale,
            y: 1180.0 / scale,
            width: 36.0 / scale,
            height: 36.0 / scale,
        };
        let p = place_windows(tray, screen, work, 1800.0);
        assert_eq!(p.width * scale, 600.0);
        assert_eq!(p.x * scale, -612.0);
        assert_eq!(p.y * scale, -188.0);
        assert_eq!((p.y + p.height) * scale, 1168.0);
        assert!(p.x >= work.x && p.x + p.width <= 0.0);
    }
}
