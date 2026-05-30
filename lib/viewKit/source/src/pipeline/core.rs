use crate::backend::{ComponentRenderer, PropertyValue, RawOSEvent, ViewKitBackend, WindowBackend};
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::io::Write;
#[cfg(feature = "wayland")]
use std::os::unix::io::AsFd;
use tiny_skia::{Color, Paint, Pixmap, Transform};
#[cfg(feature = "wayland")]
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
};
#[cfg(feature = "wayland")]
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// シンプルなコンポーネントテンプレートのキャッシュ構造
#[allow(unused)]
struct ComponentTemplate {
    name: String,
    raw: String,
    has_children_slot: bool,
    content_types: Vec<String>,
}
pub struct BackendImpl {
    width: u32,
    height: u32,
    templates: HashMap<String, ComponentTemplate>,
    // （ARGB as u32）
    pixels: Vec<u32>,
    #[cfg(feature = "wayland")]
    wayland: Option<WaylandContext>,
}

impl BackendImpl {
    pub fn new() -> Result<Self, String> {
        // NOTE: 本来はここで wayland-client / sctk を用いて Connection::connect_to_env()
        // と registry 取得、wl_shm / wl_compositor / xdg_wm_base 等を初期化します。
        // 実装は環境依存なので、まずはレンダリングパスとテンプレート処理を整備します。
        println!(
            "ViewKit: Backend initialized (Wayland connection will be established when available)"
        );

        #[cfg(feature = "wayland")]
        let wayland = match WaylandContext::new() {
            Ok(ctx) => Some(ctx),
            Err(e) => {
                eprintln!("ViewKit: Wayland disabled: {}", e);
                None
            }
        };

        Ok(Self {
            width: 800,
            height: 600,
            templates: HashMap::new(),
            pixels: vec![0u32; (800 * 600) as usize],
            #[cfg(feature = "wayland")]
            wayland,
        })
    }

    fn ensure_buffer(&mut self, width: u32, height: u32) {
        let needed = (width * height) as usize;
        if self.pixels.len() != needed {
            self.pixels.resize(needed, 0);
            self.width = width;
            self.height = height;
        }
    }

    // (helper functions follow)
}

#[cfg(feature = "wayland")]
struct WaylandState {
    running: bool,
    base_surface: Option<wl_surface::WlSurface>,
    buffer: Option<wl_buffer::WlBuffer>,
    shm_pool: Option<wl_shm_pool::WlShmPool>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    xdg_surface: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
    configured: bool,
    // swap_buffers から更新される
    file: Option<std::fs::File>,
    file_size: usize,
    mmap: Option<memmap2::MmapMut>,
    width: u32,
    height: u32,
    pending_keys: Vec<u32>,
    quit_requested: bool,
}

#[cfg(feature = "wayland")]
impl WaylandState {
    fn init_xdg_surface(&mut self, qh: &QueueHandle<WaylandState>, title: &str) {
        let wm_base = self.wm_base.as_ref().unwrap();
        let base_surface = self.base_surface.as_ref().unwrap();

        let xdg_surface = wm_base.get_xdg_surface(base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        toplevel.set_title(title.into());
        base_surface.commit();
        self.xdg_surface = Some((xdg_surface, toplevel));
    }

    fn ensure_shm_buffer(&mut self, qh: &QueueHandle<WaylandState>, w: u32, h: u32) {
        let Some(shm) = self.shm.as_ref() else { return };
        let Some(surface) = self.base_surface.as_ref() else { return };

        let stride = (w * 4) as usize;
        let size = stride.saturating_mul(h as usize);
        if size == 0 {
            return;
        }

        let need_recreate = self.buffer.is_none() || self.width != w || self.height != h;
        if !need_recreate {
            return;
        }

        // 新しいバッファを作る
        let mut file = tempfile::tempfile().expect("tempfile");
        file.set_len(size as u64).expect("set_len");
        file.flush().ok();

        let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            w as i32,
            h as i32,
            (w * 4) as i32,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );

        self.shm_pool = Some(pool);
        self.buffer = Some(buffer.clone());
        self.file = Some(file);
        self.file_size = size;
        self.mmap = None;
        self.width = w;
        self.height = h;

        // 初回は commit して map するまで待たない
        if self.configured {
            surface.attach(Some(&buffer), 0, 0);
            // wl_surface.damage_buffer は version 4 以降。互換のため damage を使う。
            surface.damage(0, 0, w as i32, h as i32);
            surface.commit();
        }
    }

    fn write_pixels(&mut self, pixels_argb: &[u32]) {
        let Some(file) = self.file.as_ref() else { return };
        if self.file_size == 0 {
            return;
        }

        if self.mmap.is_none() {
            if let Ok(m) = unsafe { memmap2::MmapOptions::new().len(self.file_size).map_mut(file) } {
                self.mmap = Some(m);
            } else {
                return;
            }
        }

        let Some(mmap) = self.mmap.as_mut() else { return };
        let needed_px = (self.width * self.height) as usize;
        if pixels_argb.len() < needed_px {
            return;
        }

        for (i, px) in pixels_argb.iter().take(needed_px).enumerate() {
            let a = ((px >> 24) & 0xFF) as u8;
            let r = ((px >> 16) & 0xFF) as u8;
            let g = ((px >> 8) & 0xFF) as u8;
            let b = (px & 0xFF) as u8;
            let off = i * 4;
            if off + 3 < mmap.len() {
                // wl_shm::Format::Argb8888 は BGRA(LE) として書くのが安全
                mmap[off] = b;
                mmap[off + 1] = g;
                mmap[off + 2] = r;
                mmap[off + 3] = a;
            }
        }
        let _ = mmap.flush();
    }
}

#[cfg(feature = "wayland")]
impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            match &interface[..] {
                "wl_compositor" => {
                    let compositor =
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ());
                    let surface = compositor.create_surface(qh, ());
                    state.base_surface = Some(surface);
                    if state.wm_base.is_some() && state.xdg_surface.is_none() {
                        state.init_xdg_surface(qh, "Kome");
                    }
                }
                "wl_shm" => {
                    let shm = registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ());
                    state.shm = Some(shm);
                    // buffer は swap_buffers でサイズが確定してから作る
                }
                "wl_seat" => {
                    registry.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, ());
                }
                "xdg_wm_base" => {
                    let wm_base = registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ());
                    state.wm_base = Some(wm_base);
                    if state.base_surface.is_some() && state.xdg_surface.is_none() {
                        state.init_xdg_surface(qh, "Kome");
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(feature = "wayland")]
delegate_noop!(WaylandState: ignore wl_compositor::WlCompositor);
#[cfg(feature = "wayland")]
delegate_noop!(WaylandState: ignore wl_surface::WlSurface);
#[cfg(feature = "wayland")]
delegate_noop!(WaylandState: ignore wl_shm::WlShm);
#[cfg(feature = "wayland")]
delegate_noop!(WaylandState: ignore wl_shm_pool::WlShmPool);
#[cfg(feature = "wayland")]
delegate_noop!(WaylandState: ignore wl_buffer::WlBuffer);

#[cfg(feature = "wayland")]
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WaylandState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

#[cfg(feature = "wayland")]
impl Dispatch<xdg_surface::XdgSurface, ()> for WaylandState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
            if let (Some(surface), Some(buffer)) = (state.base_surface.as_ref(), state.buffer.as_ref())
            {
                surface.attach(Some(buffer), 0, 0);
                // wl_surface.damage_buffer は version 4 以降。互換のため damage を使う。
                surface.damage(0, 0, state.width as i32, state.height as i32);
                surface.commit();
            }
        }
    }
}

#[cfg(feature = "wayland")]
impl Dispatch<xdg_toplevel::XdgToplevel, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.running = false;
            state.quit_requested = true;
        }
    }
}

#[cfg(feature = "wayland")]
impl Dispatch<wl_seat::WlSeat, ()> for WaylandState {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event {
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
        }
    }
}

#[cfg(feature = "wayland")]
impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key { key, state: key_state, .. } = event {
            // pressed のときだけ通知（ViewKitApp 側が pressed:true を見る）
            if matches!(key_state, WEnum::Value(wl_keyboard::KeyState::Pressed)) {
                state.pending_keys.push(key);
            }
        }
    }
}

#[cfg(feature = "wayland")]
struct WaylandContext {
    conn: Connection,
    event_queue: EventQueue<WaylandState>,
    qh: QueueHandle<WaylandState>,
    state: WaylandState,
}

#[cfg(feature = "wayland")]
impl WaylandContext {
    fn new() -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();
        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = WaylandState {
            running: true,
            base_surface: None,
            buffer: None,
            shm_pool: None,
            shm: None,
            wm_base: None,
            xdg_surface: None,
            configured: false,
            file: None,
            file_size: 0,
            mmap: None,
            width: 0,
            height: 0,
            pending_keys: Vec::new(),
            quit_requested: false,
        };

        // 初期化のため、registry と初回 configure を受け取る
        let _ = event_queue.roundtrip(&mut state);
        let _ = event_queue.roundtrip(&mut state);

        Ok(Self {
            conn,
            event_queue,
            qh,
            state,
        })
    }

    fn dispatch_pending(&mut self) {
        // 非ブロッキングで Wayland ソケットを読む
        let _ = self.conn.flush();
        if let Some(guard) = self.conn.prepare_read() {
            let _ = guard.read();
        }
        let _ = self.event_queue.dispatch_pending(&mut self.state);
        let _ = self.conn.flush();
    }
}

/// 簡易的な色文字列 (#RRGGBB) を ARGB(u32) に変換
fn parse_color_hex(s: &str) -> u32 {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() == 6 {
        if let Ok(v) = u32::from_str_radix(s, 16) {
            // ARGB (opaque)
            return 0xFF000000u32 | v;
        }
    }
    0xFF000000u32 // default: black
}

/// 再帰的に layout を計算してピクセルに描画する。（`self` を借用しない自由関数）
fn render_node_draw(pixmap: &mut Pixmap, node: &UiNode, x: i32, y: i32, width: i32, height: i32) {
    let mut paint = Paint::default();
    if let Some(color) = node.props.get("color") {
        if let Value::String(s) = color {
            let argb = parse_color_hex(s);
            let r = ((argb >> 16) & 0xFF) as u8;
            let g = ((argb >> 8) & 0xFF) as u8;
            let b = (argb & 0xFF) as u8;
            paint.set_color(Color::from_rgba8(r, g, b, 255));
        }
    } else {
        paint.set_color(Color::from_rgba8(0xEE, 0xEE, 0xEE, 255));
    }

    let rect = tiny_skia::Rect::from_xywh(x as f32, y as f32, width as f32, height as f32).unwrap();
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);

    // 子要素は縦に積む
    if !node.children.is_empty() {
        let child_h = (height as usize / node.children.len()) as i32;
        for (i, child) in node.children.iter().enumerate() {
            let cy = y + i as i32 * child_h;
            render_node_draw(pixmap, child, x + 4, cy + 4, width - 8, child_h - 8);
        }
    } else {
        let has_textish_content = node.props.get("text").is_some()
            || matches!(node.content.as_ref().map(|c| c.ty.as_str()), Some("String"));
        if has_textish_content {
            let mut label_paint = Paint::default();
            label_paint.set_color(Color::from_rgba8(0x11, 0x11, 0x11, 255));
            let lw = (width as f32 * 0.6).max(8.0);
            let lh = 18.0f32.min(height as f32 * 0.5);
            let lx = x as f32 + 8.0;
            let ly = y as f32 + (height as f32 - lh) / 2.0;
            let lrect = tiny_skia::Rect::from_xywh(lx, ly, lw, lh).unwrap();
            pixmap.fill_rect(lrect, &label_paint, Transform::identity(), None);
        }
    }
}

/// 内部的な UI ノード表現（JSON から復元）
#[derive(Debug, Clone)]
#[allow(unused)]
struct UiNode {
    id: Option<String>,
    component: String,
    props: serde_json::Map<String, Value>,
    content: Option<UiContent>,
    children: Vec<UiNode>,
}

#[derive(Debug, Clone)]
#[allow(unused)]
struct UiContent {
    ty: String,
    value: String,
}

impl UiNode {
    fn from_value(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let obj = v.as_object().unwrap();
        let component = obj
            .get("component")
            .and_then(|c| c.as_str())
            .unwrap_or("div")
            .to_string();
        let id = obj
            .get("id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let props = obj
            .get("props")
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();
        let content = obj.get("content").and_then(|c| c.as_object()).and_then(|m| {
            let ty = m.get("type").and_then(|v| v.as_str())?;
            let value = m.get("value").and_then(|v| v.as_str())?;
            Some(UiContent {
                ty: ty.to_string(),
                value: value.to_string(),
            })
        });
        let mut children = Vec::new();
        if let Some(arr) = obj.get("children").and_then(|c| c.as_array()) {
            for child in arr.iter() {
                if let Some(n) = UiNode::from_value(child) {
                    children.push(n);
                }
            }
        }
        Some(UiNode {
            id,
            component,
            props,
            content,
            children,
        })
    }
}

impl WindowBackend for BackendImpl {
    fn create_window(&mut self, width: u32, height: u32, title: &str, no_decoration: bool) {
        self.width = width;
        self.height = height;
        self.ensure_buffer(width, height);
        let _ = no_decoration;

        #[cfg(feature = "wayland")]
        {
            if let Some(ctx) = self.wayland.as_mut() {
                // registry の受信で surface/xdg が揃うまでイベントを回す
                for _ in 0..4 {
                    ctx.dispatch_pending();
                }
                if let Some((_, toplevel)) = ctx.state.xdg_surface.as_ref() {
                    toplevel.set_title(title.into());
                }
                if let Some(surface) = ctx.state.base_surface.as_ref() {
                    surface.commit();
                }
                let _ = ctx.conn.flush();
                println!("ViewKit: created Wayland window '{}' {}x{}", title, width, height);
                return;
            }
        }

        // Wayland が使えない場合のフォールバック（主にテスト用）
        println!("ViewKit: Wayland not available, window is headless");
    }

    fn swap_buffers(&mut self, buffer: &[u32], width: u32, height: u32) {
        self.ensure_buffer(width, height);

        #[cfg(feature = "wayland")]
        {
            if let Some(ctx) = self.wayland.as_mut() {
                ctx.dispatch_pending();
                ctx.state.ensure_shm_buffer(&ctx.qh, width, height);
                ctx.state.write_pixels(buffer);

                if ctx.state.configured {
                    if let (Some(surface), Some(wlbuf)) =
                        (ctx.state.base_surface.as_ref(), ctx.state.buffer.as_ref())
                    {
                        surface.attach(Some(wlbuf), 0, 0);
                        // wl_surface.damage_buffer は version 4 以降。互換のため damage を使う。
                        surface.damage(0, 0, width as i32, height as i32);
                        surface.commit();
                    }
                }
                let _ = ctx.conn.flush();
                return;
            }
        }

        // Wayland が無い場合は headless で保持だけする（PNGはデフォルトで出さない）
        self.pixels.copy_from_slice(buffer);
    }

    fn poll_os_event(&mut self) -> Option<RawOSEvent> {
        #[cfg(feature = "wayland")]
        {
            if let Some(ctx) = self.wayland.as_mut() {
                ctx.dispatch_pending();

                if ctx.state.quit_requested {
                    return Some(RawOSEvent::Quit);
                }

                if !ctx.state.pending_keys.is_empty() {
                    let key = ctx.state.pending_keys.remove(0);
                    return Some(RawOSEvent::Key {
                        scan_code: key,
                        pressed: true,
                    });
                }
            }
        }

        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ComponentRenderer for BackendImpl {
    fn register_component(&mut self, name: &str, template_html: &str) -> Result<(), String> {
        // 簡易的にタグ名と <children /> があるかを検出して保存
        let has_children = template_html.contains("<children")
            || template_html.contains("<slot")
            || template_html.contains("{children}");

        // detect content types declared in the template
        // patterns: data-content-type="foo" or <content type="foo">
        let mut content_types = Vec::new();
        // search for data-content-type="..."
        let mut search_idx = 0usize;
        while let Some(p) = template_html[search_idx..].find("data-content-type=\"") {
            let start = search_idx + p + "data-content-type=\"".len();
            if let Some(end_rel) = template_html[start..].find('"') {
                let val = &template_html[start..start + end_rel];
                content_types.push(val.to_string());
                search_idx = start + end_rel + 1;
                continue;
            } else {
                break;
            }
        }
        // search for <content ... type="...">
        search_idx = 0;
        while let Some(p) = template_html[search_idx..].find("<content") {
            let start = search_idx + p;
            if let Some(tp_pos) = template_html[start..].find("type=\"") {
                let tstart = start + tp_pos + "type=\"".len();
                if let Some(tend_rel) = template_html[tstart..].find('"') {
                    let val = &template_html[tstart..tstart + tend_rel];
                    content_types.push(val.to_string());
                    search_idx = tstart + tend_rel + 1;
                    continue;
                }
            }
            search_idx = start + 8;
        }
        content_types.sort();
        content_types.dedup();

        let tpl = ComponentTemplate {
            name: name.to_string(),
            raw: template_html.to_string(),
            has_children_slot: has_children,
            content_types: content_types.clone(),
        };
        self.templates.insert(name.to_string(), tpl);
        println!(
            "ViewKit: Registered component '{}' (children_slot={})",
            name, has_children
        );
        if !content_types.is_empty() {
            println!("ViewKit: component '{}' content_types={:?}", name, content_types);
        }
        Ok(())
    }

    fn update_ui_tree(&mut self, tree_delta_json: &str) {
        // JSON -> UiNode tree
        match serde_json::from_str::<Value>(tree_delta_json) {
            Ok(v) => {
                if let Some(root_node) = UiNode::from_value(&v) {
                    // Prepare pixmap
                    let mut pixmap =
                        Pixmap::new(self.width, self.height).expect("pixmap alloc");
                    // background
                    let mut bg_paint = Paint::default();
                    bg_paint.set_color(Color::from_rgba8(0xFF, 0xFF, 0xFF, 255));
                    let full =
                        tiny_skia::Rect::from_xywh(0.0, 0.0, self.width as f32, self.height as f32)
                            .unwrap();
                    pixmap.fill_rect(full, &bg_paint, Transform::identity(), None);

                    // render
                    render_node_draw(
                        &mut pixmap,
                        &root_node,
                        0,
                        0,
                        self.width as i32,
                        self.height as i32,
                    );

                    // copy pixmap to pixels (RGBA bytes -> ARGB u32)
                    let w = self.width as usize;
                    let h = self.height as usize;
                    let data = pixmap.data();
                    for yy in 0..h {
                        for xx in 0..w {
                            let i = (yy * w + xx) * 4;
                            let r = data[i];
                            let g = data[i + 1];
                            let b = data[i + 2];
                            let a = data[i + 3];
                            let argb = ((a as u32) << 24)
                                | ((r as u32) << 16)
                                | ((g as u32) << 8)
                                | (b as u32);
                            let idx = yy * w + xx;
                            self.pixels[idx] = argb;
                        }
                    }

                    // 最後に swap_buffers を呼ぶ (ここでは self.pixels をクローンして渡すことで
                    // 借用競合を避ける簡易実装)
                    let outbuf = self.pixels.clone();
                    self.swap_buffers(&outbuf, self.width, self.height);
                } else {
                    eprintln!("ViewKit: Failed to parse UI JSON into node");
                }
            }
            Err(e) => {
                eprintln!("ViewKit: update_ui_tree - invalid json: {}", e);
            }
        }
    }

    fn set_component_property(&mut self, _component_id: &str, _key: &str, _value: PropertyValue) {}
}

impl ViewKitBackend for BackendImpl {}
