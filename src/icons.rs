use std::collections::HashMap;

use image::{DynamicImage, RgbaImage};
use ratatui::{Frame, layout::Rect};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg::{Options, Tree},
};

use crate::k8s::ResourceKind;

/// Every rasterized icon is a square this many pixels on a side —
/// comfortably higher resolution than any tile will actually render at
/// (a handful of terminal cells), so `StatefulImage`'s own downscale
/// always has real detail to work from rather than upscaling a blurry
/// source.
const RENDER_SIZE: u32 = 128;

/// Each resource kind's official Kubernetes icon — vendored from
/// `kubernetes/community`'s icon set (see `assets/icons/ATTRIBUTION.md`),
/// keyed by that project's own short filename so the cache key and the
/// embedded bytes can't drift apart. Every CRD kind shares the one
/// generic "crd" icon — there's no per-CRD official icon to use instead.
fn icon_asset(kind: ResourceKind) -> (&'static str, &'static [u8]) {
    match kind {
        ResourceKind::Nodes => ("node", include_bytes!("../assets/icons/node.svg")),
        ResourceKind::Namespaces => ("ns", include_bytes!("../assets/icons/ns.svg")),
        ResourceKind::Pods => ("pod", include_bytes!("../assets/icons/pod.svg")),
        ResourceKind::Deployments => ("deploy", include_bytes!("../assets/icons/deploy.svg")),
        ResourceKind::ReplicaSets => ("rs", include_bytes!("../assets/icons/rs.svg")),
        ResourceKind::StatefulSets => ("sts", include_bytes!("../assets/icons/sts.svg")),
        ResourceKind::DaemonSets => ("ds", include_bytes!("../assets/icons/ds.svg")),
        ResourceKind::Jobs => ("job", include_bytes!("../assets/icons/job.svg")),
        ResourceKind::CronJobs => ("cronjob", include_bytes!("../assets/icons/cronjob.svg")),
        ResourceKind::ConfigMaps => ("cm", include_bytes!("../assets/icons/cm.svg")),
        ResourceKind::Secrets => ("secret", include_bytes!("../assets/icons/secret.svg")),
        ResourceKind::Hpas => ("hpa", include_bytes!("../assets/icons/hpa.svg")),
        ResourceKind::Services | ResourceKind::PortForwards => ("svc", include_bytes!("../assets/icons/svc.svg")),
        ResourceKind::Endpoints => ("ep", include_bytes!("../assets/icons/ep.svg")),
        ResourceKind::Ingresses => ("ing", include_bytes!("../assets/icons/ing.svg")),
        ResourceKind::NetworkPolicies => ("netpol", include_bytes!("../assets/icons/netpol.svg")),
        ResourceKind::Pvcs => ("pvc", include_bytes!("../assets/icons/pvc.svg")),
        ResourceKind::Pvs => ("pv", include_bytes!("../assets/icons/pv.svg")),
        ResourceKind::StorageClasses => ("sc", include_bytes!("../assets/icons/sc.svg")),
        ResourceKind::ServiceAccounts => ("sa", include_bytes!("../assets/icons/sa.svg")),
        ResourceKind::Roles => ("role", include_bytes!("../assets/icons/role.svg")),
        ResourceKind::RoleBindings => ("rb", include_bytes!("../assets/icons/rb.svg")),
        ResourceKind::ClusterRoles => ("c-role", include_bytes!("../assets/icons/c-role.svg")),
        ResourceKind::ClusterRoleBindings => ("crb", include_bytes!("../assets/icons/crb.svg")),
        ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) | ResourceKind::CustomResource(_, _) => {
            ("crd", include_bytes!("../assets/icons/crd.svg"))
        }
        // Overview's tile isn't drawn with an icon at all (see `IconCache::draw`'s
        // caller), so this arm is never actually reached — a fallback is still
        // required since `icon_asset` is total over `ResourceKind`.
        ResourceKind::Overview => ("pod", include_bytes!("../assets/icons/pod.svg")),
    }
}

/// Parses and rasterizes one SVG onto a square, transparent `RENDER_SIZE`
/// canvas, scaled uniformly (not stretched) to fit and centered — every
/// vendored icon is close to square already, but this keeps a
/// non-square one from distorting instead of just being letterboxed.
fn rasterize(svg: &[u8]) -> Option<DynamicImage> {
    let tree = Tree::from_data(svg, &Options::default()).ok()?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w <= 0.0 || h <= 0.0 {
        return None;
    }

    let scale = RENDER_SIZE as f32 / w.max(h);
    let tx = (RENDER_SIZE as f32 - w * scale) / 2.0;
    let ty = (RENDER_SIZE as f32 - h * scale) / 2.0;
    let transform = Transform::from_scale(scale, scale).post_translate(tx, ty);

    let mut pixmap = Pixmap::new(RENDER_SIZE, RENDER_SIZE)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let rgba = pixmap.take_demultiplied();
    let image = RgbaImage::from_raw(RENDER_SIZE, RENDER_SIZE, rgba)?;
    Some(DynamicImage::ImageRgba8(image))
}

/// Rasterizes and caches one `StatefulProtocol` per distinct icon asset,
/// lazily — the first time that resource kind's tile is actually drawn,
/// not all ~25 up front. The underlying image never changes once cached,
/// so this is a one-time cost per icon actually seen, not a per-frame one
/// (`StatefulImage` just re-fits the cached protocol into whatever `Rect`
/// it's rendered into that frame, which is the same fixed tile size every
/// time anyway).
pub struct IconCache {
    picker: Picker,
    protocols: HashMap<&'static str, StatefulProtocol>,
}

impl IconCache {
    /// `Picker::from_query_stdio` detects the terminal's actual graphics
    /// capability (Kitty/Sixel/iTerm2) by writing an escape sequence and
    /// reading the response — must run after raw mode is enabled (so it's
    /// called from `run`, not `main`, before the event-read loop starts,
    /// so it can't race with crossterm's own stdin reads). Falls back to
    /// the pure-Rust halfblocks renderer (always available, no querying)
    /// rather than failing startup if detection itself errors out, e.g.
    /// stdio isn't a real TTY.
    pub fn detect() -> Self {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        IconCache { picker, protocols: HashMap::new() }
    }

    /// A cache that never queries the terminal, for tests.
    #[cfg(test)]
    pub fn halfblocks() -> Self {
        IconCache { picker: Picker::halfblocks(), protocols: HashMap::new() }
    }

    /// Centers a roughly-square sub-area within `area` using the
    /// terminal's real font aspect ratio — tiles are sized for text
    /// (wide), so rendering an icon into the whole area would letterbox
    /// it down to a thin, hard-to-recognize sliver instead of a
    /// reasonably-sized square.
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

    fn protocol_for(&mut self, kind: ResourceKind) -> Option<&mut StatefulProtocol> {
        let (key, svg) = icon_asset(kind);
        if !self.protocols.contains_key(key) {
            let image = rasterize(svg)?;
            self.protocols.insert(key, self.picker.new_resize_protocol(image));
        }
        self.protocols.get_mut(key)
    }

    /// Draws `kind`'s icon into `area`. A silent no-op if rasterizing
    /// that SVG ever failed (it won't, for the vendored set, but a
    /// hard-coded asset table has no user-facing way to fail otherwise) —
    /// callers just get an icon-less tile rather than a crash.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, kind: ResourceKind) {
        if let Some(protocol) = self.protocol_for(kind) {
            frame.render_stateful_widget(StatefulImage::default(), area, protocol);
        }
    }
}
