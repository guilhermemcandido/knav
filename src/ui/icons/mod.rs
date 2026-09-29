use std::collections::HashMap;

use image::{DynamicImage, RgbaImage};
use ratatui::{Frame, layout::Rect};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg::{Options, Tree},
};

use crate::k8s::ResourceKind;

/// Icons are rasterized this many pixels square, well above tile size, so downscaling
/// keeps detail.
const RENDER_SIZE: u32 = 128;

/// Each kind's icon, vendored from `kubernetes/community` (see `svg/ATTRIBUTION.md`).
/// Every CRD shares the "crd" icon.
fn icon_asset(kind: ResourceKind) -> (&'static str, &'static [u8]) {
    match kind {
        ResourceKind::Nodes => ("node", include_bytes!("svg/node.svg")),
        ResourceKind::Namespaces => ("ns", include_bytes!("svg/ns.svg")),
        ResourceKind::Pods => ("pod", include_bytes!("svg/pod.svg")),
        ResourceKind::Deployments => ("deploy", include_bytes!("svg/deploy.svg")),
        ResourceKind::ReplicaSets => ("rs", include_bytes!("svg/rs.svg")),
        ResourceKind::StatefulSets => ("sts", include_bytes!("svg/sts.svg")),
        ResourceKind::DaemonSets => ("ds", include_bytes!("svg/ds.svg")),
        ResourceKind::Jobs => ("job", include_bytes!("svg/job.svg")),
        ResourceKind::CronJobs => ("cronjob", include_bytes!("svg/cronjob.svg")),
        ResourceKind::ConfigMaps => ("cm", include_bytes!("svg/cm.svg")),
        ResourceKind::Secrets => ("secret", include_bytes!("svg/secret.svg")),
        ResourceKind::Hpas => ("hpa", include_bytes!("svg/hpa.svg")),
        ResourceKind::Services | ResourceKind::PortForwards => ("svc", include_bytes!("svg/svc.svg")),
        ResourceKind::Endpoints => ("ep", include_bytes!("svg/ep.svg")),
        ResourceKind::Ingresses => ("ing", include_bytes!("svg/ing.svg")),
        ResourceKind::NetworkPolicies => ("netpol", include_bytes!("svg/netpol.svg")),
        ResourceKind::Pvcs => ("pvc", include_bytes!("svg/pvc.svg")),
        ResourceKind::Pvs => ("pv", include_bytes!("svg/pv.svg")),
        ResourceKind::StorageClasses => ("sc", include_bytes!("svg/sc.svg")),
        ResourceKind::ServiceAccounts => ("sa", include_bytes!("svg/sa.svg")),
        ResourceKind::Roles => ("role", include_bytes!("svg/role.svg")),
        ResourceKind::RoleBindings => ("rb", include_bytes!("svg/rb.svg")),
        ResourceKind::ClusterRoles => ("c-role", include_bytes!("svg/c-role.svg")),
        ResourceKind::ClusterRoleBindings => ("crb", include_bytes!("svg/crb.svg")),
        ResourceKind::CustomResourceList
        | ResourceKind::CustomResourceGroup(_)
        | ResourceKind::CustomResource(_, _)
        | ResourceKind::ApiResources
        | ResourceKind::Api(_, _)
        | ResourceKind::HelmReleases
        | ResourceKind::ExtensionDashboard(_) => ("crd", include_bytes!("svg/crd.svg")),
        // The command line shows the Overview as a house.
        ResourceKind::Overview => ("home", include_bytes!("svg/home.svg")),
    }
}

/// Icons that are not a resource kind, by name.
fn named_asset(name: &str) -> Option<(&'static str, &'static [u8])> {
    Some(match name {
        "home" => ("home", include_bytes!("svg/home.svg")),
        "door" => ("door", include_bytes!("svg/door.svg")),
        "bell" => ("bell", include_bytes!("svg/bell.svg")),
        "switch" => ("switch", include_bytes!("svg/switch.svg")),
        "palette" => ("palette", include_bytes!("svg/palette.svg")),
        "gear" => ("gear", include_bytes!("svg/gear.svg")),
        _ => return None,
    })
}

/// One SVG on a square transparent canvas, scaled evenly and centred so non-square
/// icons don't distort.
fn rasterize(svg: &[u8], fill: f32) -> Option<DynamicImage> {
    let tree = Tree::from_data(svg, &Options::default()).ok()?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w <= 0.0 || h <= 0.0 {
        return None;
    }

    // `fill` is the share of the square the icon takes.
    let scale = RENDER_SIZE as f32 * fill / w.max(h);
    let tx = (RENDER_SIZE as f32 - w * scale) / 2.0;
    let ty = (RENDER_SIZE as f32 - h * scale) / 2.0;
    let transform = Transform::from_scale(scale, scale).post_translate(tx, ty);

    let mut pixmap = Pixmap::new(RENDER_SIZE, RENDER_SIZE)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let rgba = pixmap.take_demultiplied();
    let image = RgbaImage::from_raw(RENDER_SIZE, RENDER_SIZE, rgba)?;
    Some(DynamicImage::ImageRgba8(image))
}

/// Rasterizes and caches one `StatefulProtocol` per icon, lazily on first draw.
pub struct IconCache {
    picker: Picker,
    /// Keyed by asset and fill percent, so a smaller icon is its own image.
    protocols: HashMap<(&'static str, u8), StatefulProtocol>,
    /// Off by `ui.icons = false`: nothing is drawn and layouts drop the room kept for icons.
    enabled: bool,
}

impl IconCache {
    /// Detects Kitty, Sixel or iTerm2 support by querying the terminal, so it must run
    /// in raw mode. Falls back to half blocks without a TTY.
    pub fn detect() -> Self {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        IconCache { picker, protocols: HashMap::new(), enabled: true }
    }

    /// A cache that never queries the terminal, for tests.
    #[cfg(test)]
    pub fn halfblocks() -> Self {
        IconCache { picker: Picker::halfblocks(), protocols: HashMap::new(), enabled: true }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// A roughly square part of `area` given the font's aspect ratio, so icons don't
    /// squash.
    pub fn centered_square(&self, area: Rect) -> Rect {
        let font = self.picker.font_size();
        if font.width == 0 || area.height == 0 {
            return area;
        }
        let target_width = ((area.height as f32 * font.height as f32) / font.width as f32).round() as u16;
        let width = target_width.clamp(1, area.width);
        let x = area.x + (area.width - width) / 2;
        Rect { x, y: area.y, width, height: area.height }
    }

    fn protocol_for(&mut self, key: &'static str, svg: &'static [u8], fill: f32) -> Option<&mut StatefulProtocol> {
        let slot = (key, (fill * 100.0) as u8);
        if !self.protocols.contains_key(&slot) {
            let image = rasterize(svg, fill)?;
            self.protocols.insert(slot, self.picker.new_resize_protocol(image));
        }
        self.protocols.get_mut(&slot)
    }

    /// Draws `kind`'s icon into `area`; nothing if rasterizing failed.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, kind: ResourceKind) {
        self.draw_kind(frame, area, kind, 1.0);
    }

    /// `draw` with the icon filling `fill` (0 to 1) of its square.
    pub fn draw_kind(&mut self, frame: &mut Frame, area: Rect, kind: ResourceKind, fill: f32) {
        let (key, svg) = icon_asset(kind);
        if let Some(protocol) = self.protocol_for(key, svg, fill) {
            frame.render_stateful_widget(StatefulImage::default(), area, protocol);
        }
    }

    /// A named icon (`door`, `bell`, `switch`, `home`) instead of a kind's.
    pub fn draw_named(&mut self, frame: &mut Frame, area: Rect, name: &str, fill: f32) {
        if let Some((key, svg)) = named_asset(name)
            && let Some(protocol) = self.protocol_for(key, svg, fill)
        {
            frame.render_stateful_widget(StatefulImage::default(), area, protocol);
        }
    }
}

#[cfg(test)]
mod debug_dump {
    use super::*;

    /// Not a real test: dumps icons to PNG to inspect by eye.
    /// `cargo test dump_icons -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dump_icons() {
        for kind in [ResourceKind::Pods, ResourceKind::Nodes, ResourceKind::ConfigMaps, ResourceKind::Services, ResourceKind::Deployments] {
            let (key, svg) = icon_asset(kind);
            let full = rasterize(svg, 1.0).unwrap();
            full.save(format!("/tmp/icon-{key}-full.png")).unwrap();
            let tiny = full.resize_exact(8, 16, image::imageops::FilterType::Lanczos3);
            tiny.save(format!("/tmp/icon-{key}-tile.png")).unwrap();
        }
    }
}
