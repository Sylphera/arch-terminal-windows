// Windows Terminal renderer adapted from the 3D point-cloud algorithm in
// areofyl/fetch (MIT). It preserves the Arch shape, two-axis rotation,
// shading and startup-until-input behavior without requiring WSL.
use std::ffi::c_void;
use std::io::{self, Write};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const ARCH: [&str; 19] = [
    "                  -`",
    "                 .o+`",
    "                `ooo/",
    "               `+oooo:",
    "              `+oooooo:",
    "              -+oooooo+:",
    "            `/:-:++oooo+:",
    "           `/++++/+++++++:",
    "          `/++++++++++++++:",
    "         `/+++oooooooooooo/`",
    "        ./ooosssso++osssssso+`",
    "       .oossssso-````/ossssss+`",
    "      -osssssso.      :ssssssso.",
    "     :osssssss/        osssso+++.",
    "    /ossssssss/        +ssssooo/-",
    "  `/ossssso+/:-        -:/+osssso+-",
    " `+sso+:-`                 `.-/+oso:",
    "`++:.                           `-/+/",
    ".`                                 `/",
];

// Each terminal character is rasterized at 2x2 resolution. Partial quadrants
// preserve fine Arch contours while fully covered cells retain shaded volume.
const SUB_COLS: usize = 2;
const SUB_ROWS: usize = 2;
const QUADRANTS: [&str; 16] = [
    " ", "▘", "▝", "▀", "▖", "▌", "▞", "▛",
    "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];

#[link(name = "Kernel32")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> *mut c_void;
    fn PeekConsoleInputW(handle: *mut c_void, buffer: *mut InputRecord, len: u32, read: *mut u32) -> i32;
    fn GetConsoleScreenBufferInfo(handle: *mut c_void, info: *mut ScreenBufferInfo) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputRecord {
    event_type: u16,
    padding: u16,
    key_down: i32,
    repeat: u16,
    virtual_key: u16,
    scan_code: u16,
    unicode_char: u16,
    control_state: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ScreenBufferInfo {
    size_x: i16,
    size_y: i16,
    cursor_x: i16,
    cursor_y: i16,
    attributes: u16,
    window_left: i16,
    window_top: i16,
    window_right: i16,
    window_bottom: i16,
    maximum_x: i16,
    maximum_y: i16,
}

fn key_pending() -> bool {
    let input = unsafe { GetStdHandle((-10i32) as u32) };
    if input.is_null() || input as isize == -1 { return false; }
    let mut records = [InputRecord::default(); 32];
    let mut n = 0u32;
    if unsafe { PeekConsoleInputW(input, records.as_mut_ptr(), records.len() as u32, &mut n) } == 0 {
        return false;
    }
    records[..n as usize].iter().any(|event| {
        if event.event_type != 1 || event.key_down == 0 { return false; }
        !matches!(event.virtual_key, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C)
    })
}

fn terminal_dimensions() -> (usize, usize) {
    let output = unsafe { GetStdHandle((-11i32) as u32) };
    let mut screen = ScreenBufferInfo::default();
    if unsafe { GetConsoleScreenBufferInfo(output, &mut screen) } == 0 { return (130, 40); }
    (
        usize::try_from((screen.window_right - screen.window_left + 1).max(1)).unwrap_or(130),
        usize::try_from((screen.window_bottom - screen.window_top + 1).max(1)).unwrap_or(40),
    )
}

fn weight(c: char) -> f32 {
    match c {
        'M' => 1.0, 'N' => 0.88, 'm' => 0.76,
        'd' => 0.66, 'h' | 'b' => 0.56, 'y' => 0.46,
        'o' | 'n' => 0.38, 's' => 0.30, '+' | '=' => 0.22,
        ':' => 0.18, '-' => 0.14, '`' => 0.08,
        '.' => 0.10, '/' => 0.12, '\'' => 0.06,
        ' ' => 0.0, _ => 0.15,
    }
}

#[derive(Clone, Copy)]
struct Point {
    x: f32, y: f32, z: f32,
    nx: f32, ny: f32, nz: f32,
}

fn points() -> Vec<Point> {
    let width = ARCH.iter().map(|s| s.chars().count()).max().unwrap();
    let height = ARCH.len();
    let mut heights = vec![0.0f32; width * height];
    for (r, row) in ARCH.iter().enumerate() {
        for (c, ch) in row.chars().enumerate() { heights[r * width + c] = weight(ch); }
    }
    let at = |r: isize, c: isize| -> f32 {
        if r < 0 || c < 0 || r >= height as isize || c >= width as isize { 0.0 }
        else { heights[r as usize * width + c as usize] }
    };
    let (cx, cy) = ((width as f32 - 1.0) * 0.5, (height as f32 - 1.0) * 0.5);
    let (sx, sy) = (0.07f32, 0.14f32);
    let mut result = Vec::with_capacity(20_000);
    for r in 0..height {
        for c in 0..width {
            let h = heights[r * width + c];
            if h == 0.0 { continue; }
            let dr = r as isize;
            let dc = c as isize;
            let dhdx = (at(dr, dc + 1) - at(dr, dc - 1)) * 0.5 / sx;
            let dhdy = (at(dr + 1, dc) - at(dr - 1, dc)) * 0.5 / sy;
            let (mut nx, mut ny, mut nz) = (-dhdx, dhdy, 1.0f32);
            let inv = (nx * nx + ny * ny + nz * nz).sqrt().recip();
            nx *= inv; ny *= inv; nz *= inv;
            let mut edge = false;
            let (mut ex, mut ey) = (0.0f32, 0.0f32);
            for rr in -1..=1isize { for cc in -1..=1isize {
                if rr == 0 && cc == 0 { continue; }
                let neighbor = at(dr + rr, dc + cc);
                if neighbor <= 0.0 { edge = true; }
                if neighbor < h { ex += cc as f32; ey -= rr as f32; }
            }}
            let il = (ex*ex + ey*ey).sqrt();
            if il > 0.00001 { ex /= il; ey /= il; }
            let layers = if edge || h < 0.15 { 2 } else { 6 };
            // Match the denser output grid to avoid sparse, jagged outlines.
            const SUBDIV: usize = 4;
            for sr in 0..SUBDIV { for sc in 0..SUBDIV {
                let fr = sr as f32 / SUBDIV as f32;
                let fc = sc as f32 / SUBDIV as f32;
                let h00 = h;
                let h10 = at(dr + 1, dc);
                let h01 = at(dr, dc + 1);
                let h11 = at(dr + 1, dc + 1);
                let interpolated = h00*(1.0-fr)*(1.0-fc)+h10*fr*(1.0-fc)+h01*(1.0-fr)*fc+h11*fr*fc;
                if interpolated <= 0.0 { continue; }
                let ox = (c as f32 + fc - cx) * sx;
                let oy = (cy - r as f32 - fr) * sy;
                let zr = interpolated * 0.18 * 1.7;
                for k in 0..layers {
                    let t = k as f32 / (layers-1) as f32 * 2.0 - 1.0;
                    let (xx, yy, zz) = if k == 0 { (nx, ny, -nz) }
                        else if k == layers-1 { (nx, ny, nz) }
                        else { let side = (1.0-t*t).sqrt(); (ex*side,ey*side,t) };
                    result.push(Point { x: ox, y: oy, z: t*zr, nx: xx, ny: yy, nz: zz });
                }
            }}
        }
    }
    result
}

fn fastfetch_info(shell: &str) -> Vec<String> {
    // Keep the startup presentation alongside this renderer, leaving the
    // user's regular fastfetch configuration untouched.
    let startup_config = std::env::current_exe().ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("startup.jsonc")));
    let mut command = Command::new("fastfetch");
    if let Some(path) = startup_config.filter(|path| path.is_file()) {
        command.arg("--config").arg(path);
    }
    let output = command.args(["--logo-type", "none", "--pipe", "false"]).output();
    match output {
        Ok(out) if out.status.success() => {
            let content = String::from_utf8_lossy(&out.stdout);
            let mut lines: Vec<String> = content.lines().map(|s| {
                let line = s.trim_end_matches('\r');
                if let Some(start) = line.find("fetch-arch") {
                    // Fastfetch detects this renderer as the active shell.
                    // Replace its full executable name, including variant
                    // suffixes, with the PowerShell version passed by the
                    // profile. Preserve the original terminal color codes.
                    let end = line[start..].find('\x1b').map(|n| start + n)
                        .unwrap_or(line.len());
                    format!("{}{}{}", &line[..start], shell, &line[end..])
                } else { line.to_owned() }
            }).collect();
            // Repeat the theme palette above the title. The line below it
            // now matches the single blank line above the bottom palette.
            add_top_palette(&mut lines);
            lines
        }
        _ => vec!["Windows Terminal — Arch 3D".to_owned()],
    }
}

fn add_top_palette(lines: &mut Vec<String>) {
    if let Some(palette) = lines.iter().find(|line| line.contains('●')).cloned() {
        // Fastfetch's initial blank line contains ANSI reset codes. Using
        // trim().is_empty() previously inserted a second, unwanted blank.
        if lines.first().is_some_and(|line| visible_width(line) == 0) {
            lines[0] = palette;
        } else {
            lines.insert(0, palette);
        }
        lines.insert(1, String::new());
    }
}

// Treat ANSI SGR colors as zero-width and preserve the literal text cells.
// This prevents CPU, storage and OS values from wrapping into adjacent panes.
fn next_escape_end(bytes: &[u8], index: usize) -> Option<(usize, bool)> {
    if bytes.get(index) != Some(&0x1b) { return None; }
    if bytes.get(index + 1) == Some(&b'[') {
        let mut end = index + 2;
        while end < bytes.len() {
            if (0x40..=0x7e).contains(&bytes[end]) {
                return Some((end + 1, bytes[end] == b'm'));
            }
            end += 1;
        }
        return Some((bytes.len(), false));
    }
    if bytes.get(index + 1) == Some(&b']') {
        let mut end = index + 2;
        while end < bytes.len() {
            if bytes[end] == 7 { return Some((end + 1, false)); }
            if bytes[end] == 0x1b && bytes.get(end + 1) == Some(&b'\\') {
                return Some((end + 2, false));
            }
            end += 1;
        }
        return Some((bytes.len(), false));
    }
    Some(((index + 2).min(bytes.len()), false))
}

fn char_cells(c: char) -> usize {
    let u = c as u32;
    if u < 32 || (0x300..=0x36f).contains(&u) ||
        (0xfe00..=0xfe0f).contains(&u) || (0x200b..=0x200f).contains(&u) { return 0; }
    if (0x1100..=0x115f).contains(&u) || (0x2e80..=0xa4cf).contains(&u) ||
       (0xac00..=0xd7a3).contains(&u) || (0xf900..=0xfaff).contains(&u) ||
       (0xfe10..=0xfe6f).contains(&u) || (0xff01..=0xff60).contains(&u) ||
       (0x1f300..=0x1faff).contains(&u) { 2 } else { 1 }
}

fn visible_width(s: &str) -> usize {
    let mut result = 0;
    let mut i = 0;
    while i < s.len() {
        if let Some((end, _)) = next_escape_end(s.as_bytes(), i) { i = end; continue; }
        let c = s[i..].chars().next().unwrap();
        result += char_cells(c);
        i += c.len_utf8();
    }
    result
}

fn clip_ansi(s: &str, max_width: usize) -> String {
    if max_width == 0 { return String::new(); }
    let clipped = visible_width(s) > max_width;
    let allowed = if clipped { max_width.saturating_sub(1) } else { max_width };
    let mut result = String::new();
    let mut cols = 0;
    let mut i = 0;
    while i < s.len() {
        if let Some((end, sgr)) = next_escape_end(s.as_bytes(), i) {
            if sgr { result.push_str(&s[i..end]); }
            i = end;
            continue;
        }
        let c = s[i..].chars().next().unwrap();
        if c.is_control() {
            i += c.len_utf8();
            continue;
        }
        let cells = char_cells(c);
        if cols + cells > allowed { break; }
        result.push(c);
        cols += cells;
        i += c.len_utf8();
    }
    if clipped { result.push('…'); }
    result.push_str("\x1b[0m");
    result
}

fn compact_info(info: &[String], max_rows: usize) -> Vec<String> {
    if max_rows == 0 { return Vec::new(); }
    let nonblank: Vec<&String> = info.iter()
        .filter(|line| visible_width(line.trim()) > 0).collect();
    if nonblank.len() <= max_rows { return nonblank.into_iter().cloned().collect(); }
    // Keep the user-requested specifications even in narrow terminal panes.
    // The secondary GPU and less essential metadata go last.
    let mut selected = vec![0usize];
    let last = nonblank.len().saturating_sub(1);
    let space_before_bottom = max_rows.saturating_sub(usize::from(last > 0));
    let preferred = [" OS ", " Uptime ", " CPU ", " GPU ", " Memory ",
                     " Disk ", " Shell ", " Terminal ", " Device "];
    if last > 1 && selected.len() < space_before_bottom { selected.push(1); } // Title
    for field in preferred {
        if selected.len() >= space_before_bottom { break; }
        if let Some(index) = (1..last).find(|&i| !selected.contains(&i) && nonblank[i].contains(field)) {
            selected.push(index);
        }
    }
    for candidate in 1..last {
        if selected.len() >= space_before_bottom { break; }
        if !selected.contains(&candidate) { selected.push(candidate); }
    }
    if max_rows > 1 && last > 0 { selected.push(last); }
    selected.sort_unstable();
    selected.dedup();
    selected.into_iter().take(max_rows).map(|i| nonblank[i].clone()).collect()
}

struct RenderedFrame {
    text: String,
}

// Terminal panes can resize in the middle of a frame. CRLF output at the
// bottom of a pane scrolls its viewport, and Windows Terminal can reflow that
// output while its tiling layout is still changing. Draw every row at an
// absolute position instead: no line feeds, no implicit scrolling.
fn frame_to_vt(frame: &RenderedFrame, layout: &Layout, clear: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.text.len() + layout.output_rows * 28 + 96);
    // Windows Terminal supports synchronized output; unsupported hosts ignore
    // this private mode. The complete frame is also sent as one buffer.
    out.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
    if clear { out.extend_from_slice(b"\x1b[2J"); }
    for (index, line) in frame.text.split("\r\n").enumerate() {
        if index >= layout.output_rows { break; }
        out.extend_from_slice(format!("\x1b[{};1H\x1b[0m\x1b[2K", index + 1).as_bytes());
        out.extend_from_slice(line.as_bytes());
    }
    // The final paint never touches the last two rows reserved for PSReadLine.
    out.extend_from_slice(format!("\x1b[{};1H\x1b[0m\x1b[J", layout.output_rows + 1).as_bytes());
    out.extend_from_slice(b"\x1b[?2026l");
    out
}

fn finish_vt(layout: &Layout) -> String {
    let bottom = (layout.output_rows + 2).min(layout.rows.saturating_sub(1).max(1));
    // Reposition while the cursor is still hidden, then show it for PSReadLine.
    // Explicitly end any outstanding synchronized update on the way out.
    format!("\x1b[?2026l\x1b[{};1H\x1b[0m\x1b[?25h", bottom)
}

// Stable projected bounds are computed once for the full two-axis rotation.
// Individual frames never change their pivot or screen area.
#[derive(Clone, Copy)]
struct Bounds { left: f32, right: f32, top: f32, bottom: f32 }

fn rotation_bounds(points: &[Point]) -> Bounds {
    let k1 = 37.0 * 23.0 / 36.0;
    let k2 = 5.5f32;
    let mut bounds = Bounds {
        left: f32::INFINITY, right: f32::NEG_INFINITY,
        top: f32::INFINITY, bottom: f32::NEG_INFINITY,
    };
    for frame in 0..=500 {
        let a = frame as f32 * 0.04;
        let b = frame as f32 * 0.06;
        let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
        for p in points {
            let y1 = p.y * ca - p.z * sa;
            let z1 = p.y * sa + p.z * ca;
            let x2 = p.x * cb + z1 * sb;
            let z2 = -p.x * sb + z1 * cb;
            let zc = z2 + k2;
            if zc >= 0.1 {
                let x = 2.0 * k1 * x2 / zc;
                let y = -k1 * y1 / zc;
                bounds.left = bounds.left.min(x);
                bounds.right = bounds.right.max(x);
                bounds.top = bounds.top.min(y);
                bounds.bottom = bounds.bottom.max(y);
            }
        }
    }
    bounds
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode { Side, Stacked, InfoOnly }

struct Layout {
    cols: usize, rows: usize,
    mode: Mode,
    logo_width: usize, logo_rows: usize,
    info_width: usize,
    xcenter: f32, ycenter: f32, scale: f32,
    output_rows: usize,
    info: Vec<String>,
}

fn layout_for(bounds: Bounds, original_info: &[String], cols: usize, rows: usize) -> Layout {
    // Leave the last two lines to PowerShell's two-line prompt and one guard
    // row, so Windows Terminal never scrolls while rendering the fetch.
    let available_width = cols.saturating_sub(1).max(1);
    let available_rows = rows.saturating_sub(3).max(1);
    let mode = if cols >= 76 && rows >= 14 { Mode::Side }
        else if cols >= 26 && rows >= 20 { Mode::Stacked }
        else { Mode::InfoOnly };
    let logo_width = match mode {
        Mode::Side => 56.min((available_width / 2).max(24))
            .min(available_width.saturating_sub(39)),
        Mode::Stacked => available_width.min(56),
        Mode::InfoOnly => 0,
    };
    let info_width = match mode {
        Mode::Side => available_width.saturating_sub(logo_width + 2),
        _ => available_width,
    };
    let target_logo_rows = match mode {
        Mode::Side => available_rows.min(23),
        Mode::Stacked => available_rows.saturating_sub(8).min(14).max(5),
        Mode::InfoOnly => 0,
    };
    let wanted_info_rows = match mode {
        Mode::Side => available_rows,
        Mode::Stacked => available_rows.saturating_sub(target_logo_rows + 1),
        Mode::InfoOnly => available_rows,
    };
    let info = if mode == Mode::Side && original_info.len() <= wanted_info_rows {
        original_info.to_vec()
    } else {
        compact_info(original_info, wanted_info_rows)
    };
    let mut scale = 0.0f32;
    let (mut xcenter, mut ycenter, mut logo_rows) = (0.0f32, 0.0f32, 0usize);
    if mode != Mode::InfoOnly {
        let projected_width = (bounds.right - bounds.left).max(1.0);
        let projected_height = (bounds.bottom - bounds.top).max(1.0);
        scale = (logo_width.saturating_sub(2) as f32 / projected_width)
            .min(target_logo_rows.saturating_sub(2) as f32 / projected_height)
            .min(1.0);
        xcenter = logo_width as f32 * 0.5 - (bounds.left + bounds.right) * 0.5 * scale;
        ycenter = 0.75 - bounds.top * scale;
        logo_rows = ((ycenter + bounds.bottom * scale).ceil() as usize + 1)
            .min(target_logo_rows);
    }
    let output_rows = match mode {
        Mode::Side => logo_rows.max(info.len()),
        Mode::Stacked => logo_rows + 1 + info.len(),
        Mode::InfoOnly => info.len(),
    };
    Layout {
        cols, rows, mode, logo_width, logo_rows,
        info_width, xcenter, ycenter, scale, output_rows, info,
    }
}

fn render(points: &[Point], layout: &Layout, a: f32, b: f32) -> RenderedFrame {
    if layout.mode == Mode::InfoOnly {
        return RenderedFrame {
            text: layout.info.iter().map(|line| clip_ansi(line, layout.info_width))
                .collect::<Vec<_>>().join("\r\n"),
        };
    }
    let width = layout.logo_width;
    let height = layout.logo_rows;
    let raster_width = width * SUB_COLS;
    let raster_height = height * SUB_ROWS;
    let mut depth = vec![0.0f32; raster_width * raster_height];
    let mut light = vec![0.0f32; raster_width * raster_height];
    let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
    let k1 = 37.0 * 23.0 / 36.0 * layout.scale;
    let k2 = 5.5;
    // Same directional light and Blinn-Phong specular exponent as fetch-bin.
    let (lx, ly, lz) = (0.4082f32, 0.8165f32, -0.4082f32);
    let (mut hx, mut hy, mut hz) = (lx, ly, lz-1.0);
    let hl = (hx*hx+hy*hy+hz*hz).sqrt();
    hx /= hl; hy /= hl; hz /= hl;
    for p in points {
        let y1 = p.y*ca - p.z*sa;
        let z1 = p.y*sa + p.z*ca;
        let x2 = p.x*cb + z1*sb;
        let z2 = -p.x*sb + z1*cb;
        let ny1 = p.ny*ca - p.nz*sa;
        let nz1 = p.ny*sa + p.nz*ca;
        let nx2 = p.nx*cb + nz1*sb;
        let nz2 = -p.nx*sb + nz1*cb;
        let zc = z2 + k2;
        if zc < 0.1 { continue; }
        let inv_z = 1.0 / zc;
        let x = ((layout.xcenter + 2.0*k1*x2*inv_z) * SUB_COLS as f32) as isize;
        let y = ((layout.ycenter - k1*y1*inv_z) * SUB_ROWS as f32) as isize;
        if x < 0 || y < 0 || x >= raster_width as isize || y >= raster_height as isize { continue; }
        let i = y as usize * raster_width + x as usize;
        if inv_z <= depth[i] { continue; }
        let diff = (nx2*lx + ny1*ly + nz2*lz).max(0.0);
        let spec_dot = (nx2*hx + ny1*hy + nz2*hz).max(0.0);
        let spec = spec_dot.powi(8);
        light[i] = (0.08 + 0.62*diff + 0.30*spec).min(1.0);
        depth[i] = inv_z;
    }
    let mut lines = Vec::with_capacity(height + 3 + layout.info.len());
    for y in 0..height {
        let mut line = String::with_capacity(width*4+110);
        line.push_str("\x1b[1;36m");
        for x in 0..width {
            let mut mask = 0usize;
            let mut brightness = 0.0f32;
            for sy in 0..SUB_ROWS { for sx in 0..SUB_COLS {
                let i = (y*SUB_ROWS+sy)*raster_width + (x*SUB_COLS+sx);
                if depth[i] == 0.0 { continue; }
                mask |= 1 << (sy*SUB_COLS+sx);
                brightness += light[i];
            }}
            if mask == 0 { line.push(' '); }
            else {
                if mask < 15 { line.push_str(QUADRANTS[mask]); }
                else {
                let value = brightness / 4.0;
                line.push(if value < 0.23 { '░' } else if value < 0.45 { '▒' }
                          else if value < 0.68 { '▓' } else { '█' });
                }
            }
        }
        line.push_str("\x1b[0m");
        if layout.mode == Mode::Side && y < layout.info.len() {
            line.push_str("  ");
            line.push_str(&clip_ansi(&layout.info[y], layout.info_width));
        }
        line.push_str("\x1b[K");
        lines.push(line);
    }
    // All frames end on the same row so the prompt does not jump either.
    // Side-by-side details may occupy more rows than the logo. Do not
    // truncate those details and do not let long values wrap.
    if layout.mode == Mode::Side {
        for y in height..layout.output_rows {
            lines.push(format!("{}  {}\x1b[K", " ".repeat(width),
                clip_ansi(&layout.info[y], layout.info_width)));
        }
    } else {
        lines.push(String::new());
        lines.extend(layout.info.iter().map(|line| clip_ansi(line, layout.info_width)));
    }
    RenderedFrame {
        text: lines.join("\r\n"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let max_frames = args.iter().find_map(|s| s.strip_prefix("--frames=").and_then(|v| v.parse::<usize>().ok()));
    let shell = args.iter().find_map(|s| s.strip_prefix("--shell=")).unwrap_or("PowerShell");
    let self_test = args.iter().any(|s| s == "--self-test");
    let test_size = args.iter().find_map(|arg| {
        let (w, h) = arg.strip_prefix("--test-size=")?.split_once('x')?;
        Some((w.parse::<usize>().ok()?, h.parse::<usize>().ok()?))
    });
    let pts = points();
    let bounds = rotation_bounds(&pts);
    let mut raw_info = fastfetch_info(shell);
    let (columns, rows) = test_size.unwrap_or_else(terminal_dimensions);
    let mut layout = layout_for(bounds, &raw_info, columns, rows);
    if self_test {
        println!("fetch-arch: points={}, mode={:?}, info={}, cols={}, rows={}, logo_width={}, logo_rows={}, info_width={}, output_rows={}, two_axes=true",
            pts.len(), layout.mode, layout.info.len(), columns, rows, layout.logo_width,
            layout.logo_rows, layout.info_width, layout.output_rows);
        return;
    }
    let mut output = io::stdout().lock();
    let mut count = 0usize;
    let mut last_resize = Instant::now();
    let mut next_frame = last_resize;
    let mut clear_next = true;
    loop {
        if key_pending() { break; }
        let dimensions = test_size.unwrap_or_else(terminal_dimensions);
        if dimensions != (layout.cols, layout.rows) {
            layout = layout_for(bounds, &raw_info, dimensions.0, dimensions.1);
            last_resize = Instant::now();
            clear_next = true;
        }
        // Several panes created at once are repeatedly resized by the tiler.
        // Do not paint at an intermediate size that will immediately reflow.
        if dimensions.0 < 8 || dimensions.1 < 6 ||
            last_resize.elapsed() < Duration::from_millis(90) {
            next_frame = Instant::now();
            thread::sleep(Duration::from_millis(12));
            continue;
        }
        if count > 0 && count % 40 == 0 {
            raw_info = fastfetch_info(shell);
            layout = layout_for(bounds, &raw_info, dimensions.0, dimensions.1);
        }
        let a = count as f32 * 0.04;
        let b = count as f32 * 0.06;
        let image = render(&pts, &layout, a, b);
        // The viewport may have changed while rendering; discard that frame.
        if test_size.is_none() && terminal_dimensions() != dimensions {
            last_resize = Instant::now();
            next_frame = last_resize;
            clear_next = true;
            continue;
        }
        let painted = frame_to_vt(&image, &layout, clear_next);
        if output.write_all(&painted).is_err() { break; }
        if output.flush().is_err() { break; }
        clear_next = false;
        count += 1;
        if max_frames.is_some_and(|m| count >= m) { break; }
        // Never try to replay missed frames after a resize or CPU spike.
        // Catch-up bursts were especially expensive with six open terminals.
        next_frame = next_frame.max(Instant::now()) + Duration::from_millis(50);
        if let Some(sleep) = next_frame.checked_duration_since(Instant::now()) {
            thread::sleep(sleep);
        }
    }
    // Leave the last Arch frame visible; return control and any pending keystroke
    // to PowerShell / PSReadLine, which now draws the prompt under the fetch.
    // Read the final viewport once more: a user may stop the animation during
    // a resize. Give the shell a valid prompt position at that *current* size.
    let final_dims = test_size.unwrap_or_else(terminal_dimensions);
    if final_dims != (layout.cols, layout.rows) {
        layout = layout_for(bounds, &raw_info, final_dims.0, final_dims.1);
        let image = render(&pts, &layout, count as f32 * 0.04, count as f32 * 0.06);
        let _ = output.write_all(&frame_to_vt(&image, &layout, true));
    }
    let _ = output.write_all(finish_vt(&layout).as_bytes());
    let _ = output.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info() -> Vec<String> {
        vec![
            "\x1b[32m● ● ● ● ● ● ● ●\x1b[0m".into(),
            String::new(),
            "Astra@NitroANV15-52".into(),
            String::new(),
            " 󰍹  OS        ➜ Windows 11 Home (26H2) x86_64".into(),
            "   Shell     ➜ PowerShell 7.7.0-rc.1".into(),
            "   Terminal  ➜ Windows Terminal 1.24".into(),
            " 󰅐  Uptime    ➜ 1 day, 14 hours".into(),
            String::new(),
            " 󰌢  Device    ➜ Sportage_RTH".into(),
            format!("   CPU       ➜ 13th Gen Intel Core i9-13900H {}", "CPU".repeat(35)),
            " 󰢮  GPU       ➜ NVIDIA RTX 5050 Laptop [Discrete]".into(),
            " 󰢮  GPU       ➜ Intel Iris Xe [Integrated]".into(),
            " 󰍛  Memory    ➜ 16.9 GiB / 31.7 GiB".into(),
            "   Disk      ➜ 677.3 GiB / 947.7 GiB - NTFS".into(),
            String::new(),
            "\x1b[32m● ● ● ● ● ● ● ●\x1b[0m".into(),
        ]
    }

    #[test]
    fn palette_above_title_has_one_blank_like_palette_below_hardware() {
        let palette = "\x1b[32m● ● ● ●\x1b[0m";
        let mut lines = vec!["\x1b[0m".to_owned(), "Astra@PC".to_owned(),
                             "CPU  ➜  Intel".to_owned(), String::new(), palette.to_owned()];
        add_top_palette(&mut lines);
        assert_eq!(&lines[0], palette);
        assert_eq!(&lines[1], "");
        assert_eq!(&lines[2], "Astra@PC");
        assert_eq!(&lines[3], "CPU  ➜  Intel");
        assert_eq!(&lines[4], "");
        assert_eq!(&lines[5], palette);
    }

    #[test]
    fn compact_selection_retains_main_hardware_and_uptime() {
        let sample = sample_info();
        let lines = compact_info(&sample, 9).join("\n");
        for name in ["OS", "Uptime", "CPU", "GPU", "Memory", "Disk"] {
            assert!(lines.contains(name), "compact view lost {name}");
        }
    }

    #[test]
    fn ansi_clipping_keeps_colors_and_bounds() {
        let original = "\x1b[33mOS ➜ Windows 11 Home 26H2\x1b[0m";
        for width in 1..=45 {
            let actual = clip_ansi(original, width);
            assert!(visible_width(&actual) <= width, "width={width}");
            assert!(actual.ends_with("\x1b[0m"));
        }
        assert_eq!(visible_width("▘▝▀▖▌▞▛▗▚▐▜▄▙▟█░▒▓"), 18);
        assert_eq!(visible_width(&clip_ansi("\x1b[999Cabc", 2)), 2);
    }

    #[test]
    fn resize_layout_never_wraps_or_scrolls() {
        let points = points();
        let bounds = rotation_bounds(&points);
        let original = sample_info();
        let sizes = [
            (180, 55), (160, 40), (110, 35), (100, 28),
            (95, 18), (80, 22), (76, 14), (75, 40),
            (70, 30), (55, 26), (42, 24), (32, 20),
            (36, 16), (20, 10), (12, 7),
        ];
        for (cols, rows) in sizes {
            let layout = layout_for(bounds, &original, cols, rows);
            assert!(layout.output_rows <= rows - 3,
                    "{cols}x{rows}: output rows={}", layout.output_rows);
            for (a, b) in [(0.0, 0.0), (1.3, 1.8), (3.6, 5.4)] {
                let image = render(&points, &layout, a, b);
                let lines: Vec<&str> = image.text.split("\r\n").collect();
                assert_eq!(lines.len(), layout.output_rows,
                    "{cols}x{rows}, mode={:?}", layout.mode);
                for (i, line) in lines.iter().enumerate() {
                    assert!(visible_width(line) <= cols - 1,
                        "{cols}x{rows} row={i} width={} mode={:?}",
                        visible_width(line), layout.mode);
                }
            }
        }
    }

    #[test]
    fn palettes_are_preserved_in_compact_views() {
        let sample = sample_info();
        for n in 2..=10 {
            let selection = compact_info(&sample, n);
            assert_eq!(selection.first(), Some(&sample[0]));
            assert_eq!(selection.last(), sample.last());
        }
    }

    // Interpret the small VT subset emitted by this program. Treat an attempt
    // to write the last terminal column as unsafe: a pane shrinking during a
    // paint could otherwise wrap and scroll into the shell's prompt area.
    fn check_vt_frame(stream: &[u8], cols: usize, rows: usize) -> (Vec<usize>, usize, usize) {
        let output = std::str::from_utf8(stream).expect("VT output must be UTF-8");
        let mut index = 0;
        let mut cursor_row = 1usize;
        let mut cursor_col = 1usize;
        let mut row_addresses = Vec::new();
        let mut sync_open = false;
        let mut sync_starts = 0;
        let mut sync_ends = 0;
        let mut clear_all = 0;
        let mut clear_rest = 0;
        let mut cursor_hidden = false;
        while index < output.len() {
            if output.as_bytes()[index] == 0x1b {
                assert_eq!(output.as_bytes().get(index + 1), Some(&b'['));
                let params_start = index + 2;
                let mut final_byte = params_start;
                while final_byte < output.len() &&
                    !(0x40..=0x7e).contains(&output.as_bytes()[final_byte]) {
                    final_byte += 1;
                }
                assert!(final_byte < output.len(), "unterminated ANSI sequence");
                let params = &output[params_start..final_byte];
                match (params, output.as_bytes()[final_byte]) {
                    ("?2026", b'h') => {
                        assert!(!sync_open, "nested synchronized output");
                        sync_open = true;
                        sync_starts += 1;
                    }
                    ("?2026", b'l') => {
                        assert!(sync_open, "synchronized output closed without opening");
                        sync_open = false;
                        sync_ends += 1;
                    }
                    ("?25", b'l') => {
                        assert!(sync_open);
                        cursor_hidden = true;
                    }
                    (_, b'H') => {
                        assert!(sync_open && cursor_hidden);
                        let (row, col) = params.split_once(';').expect("absolute row;col");
                        cursor_row = row.parse().expect("numeric row");
                        cursor_col = col.parse().expect("numeric column");
                        assert!((1..=rows).contains(&cursor_row));
                        assert!((1..=cols).contains(&cursor_col));
                        row_addresses.push(cursor_row);
                    }
                    ("2", b'J') => { assert!(sync_open); clear_all += 1; }
                    ("", b'J') => { assert!(sync_open); clear_rest += 1; }
                    (_, b'K' | b'm') => { assert!(sync_open); }
                    _ => panic!("unexpected VT command: ESC[{params}{}", output.as_bytes()[final_byte] as char),
                }
                index = final_byte + 1;
            } else {
                let ch = output[index..].chars().next().unwrap();
                assert!(sync_open && cursor_hidden, "paint outside synchronized frame");
                assert!(!ch.is_control(), "implicit line movement in VT frame");
                let width = char_cells(ch);
                if width > 0 {
                    assert!(cursor_col + width - 1 < cols,
                        "paint would reach last column at {cursor_row}:{cursor_col} in {cols}x{rows}");
                    cursor_col += width;
                }
                index += ch.len_utf8();
            }
        }
        assert!(!sync_open, "terminal left with synchronized output enabled");
        assert_eq!(sync_starts, 1);
        assert_eq!(sync_ends, 1);
        assert_eq!(clear_rest, 1);
        (row_addresses, clear_all, cursor_row)
    }

    #[test]
    fn absolute_vt_paint_is_balanced_and_does_not_scroll() {
        let pts = points();
        let bounds = rotation_bounds(&pts);
        let info = sample_info();
        for (cols, rows) in [
            (180, 55), (100, 28), (76, 14), (75, 40),
            (55, 26), (32, 20), (20, 10), (12, 7),
        ] {
            let layout = layout_for(bounds, &info, cols, rows);
            for clear in [true, false] {
                let frame = render(&pts, &layout, 1.3, 2.6);
                let vt = frame_to_vt(&frame, &layout, clear);
                let (addresses, clear_all, cursor_row) = check_vt_frame(&vt, cols, rows);
                assert_eq!(addresses, (1..=layout.output_rows + 1).collect::<Vec<_>>(),
                    "incorrect absolute row sequence in {cols}x{rows}");
                assert_eq!(clear_all, usize::from(clear));
                assert_eq!(cursor_row, layout.output_rows + 1);
                assert!(cursor_row <= rows - 2, "reserved prompt rows painted");
            }
        }
    }

    #[test]
    fn final_cursor_is_moved_before_being_shown() {
        let pts = points();
        let bounds = rotation_bounds(&pts);
        for (cols, rows) in [(180, 55), (76, 14), (32, 20), (12, 7)] {
            let layout = layout_for(bounds, &sample_info(), cols, rows);
            let vt = finish_vt(&layout);
            let expected_row = layout.output_rows + 2;
            assert!(expected_row < rows);
            assert!(vt.starts_with("\x1b[?2026l"));
            assert!(vt.contains(&format!("\x1b[{expected_row};1H")));
            let show = vt.find("\x1b[?25h").unwrap();
            let move_to_prompt = vt.find(&format!("\x1b[{expected_row};1H")).unwrap();
            assert!(move_to_prompt < show, "cursor shown before prompt position");
            assert!(vt.ends_with("\x1b[?25h"));
        }
    }
}
