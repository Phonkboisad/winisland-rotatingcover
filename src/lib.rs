use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use winisland_plugin_api::abi::{
    ABI_VERSION_2, CAP_EVENTS, CAP_HOST_STATE, CAP_IMAGE, CAP_SURFACE, PluginCreateInfoV2,
    PluginDescriptorV2, PluginHandleV2, PluginStatus,
};
use winisland_plugin_api::sdk::{
    CallbackResource, DrawListBuilder, Error, Host, ImageHandle, Rgba, Size, Surface,
};
use winisland_plugin_api::{PluginMetadataC, PluginToken, SurfaceSpecV2, WidgetId};

const PLUGIN_ID: &str = "rotating-cd";
const PLUGIN_NAME: &str = "Rotating CD";
const PLUGIN_AUTHOR: &str = "phonkboisad";
const PLUGIN_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const ROTATION_SECONDS: f64 = 2.5;

// These match WinIsland's native compact music artwork inset and size.
const MINI_COVER_LEFT_INSET: f32 = 10.0;
const MINI_COVER_SIZE: f32 = 18.0;
// Overhang the native rounded-square art so its corners cannot show outside the circular crop.
const DISC_DIAMETER: f32 = 22.5;

struct RenderState {
    cover: Option<ImageHandle>,
    has_media: bool,
    playing: bool,
    expanded: bool,
    angle: f32,
}

struct Instance {
    host: Host,
    surface: Option<Arc<Surface>>,
    state: Arc<Mutex<RenderState>>,
    state_events: Option<CallbackResource>,
    resize_events: Option<CallbackResource>,
}

static DESCRIPTOR: PluginDescriptorV2 = PluginDescriptorV2 {
    struct_size: std::mem::size_of::<PluginDescriptorV2>() as u32,
    abi_version: ABI_VERSION_2,
    capabilities: CAP_EVENTS | CAP_HOST_STATE | CAP_IMAGE | CAP_SURFACE,
    metadata: PluginMetadataC::new(
        PLUGIN_ID,
        PLUGIN_NAME,
        env!("CARGO_PKG_VERSION"),
        PLUGIN_AUTHOR,
        PLUGIN_DESCRIPTION,
    ),
    create: Some(create),
    shutdown: Some(shutdown),
    destroy: Some(destroy),
    on_tick: Some(on_tick),
};

unsafe extern "C" fn create(
    info: *const PluginCreateInfoV2,
    out_handle: *mut PluginHandleV2,
) -> PluginStatus {
    if info.is_null() || out_handle.is_null() {
        return PluginStatus::InvalidArgument;
    }
    // SAFETY: `out_handle` is a valid writable output pointer supplied by the host.
    unsafe { out_handle.write(std::ptr::null_mut()) };

    // SAFETY: The host supplies a readable create-info header for this callback.
    let info = unsafe { &*info };
    if info.struct_size < std::mem::size_of::<PluginCreateInfoV2>() as u32
        || info.abi_version != ABI_VERSION_2
        || info.plugin_token == PluginToken::INVALID
    {
        return PluginStatus::UnsupportedVersion;
    }

    // SAFETY: The host table remains valid for the lifetime of the plugin instance.
    let host = match unsafe { Host::from_raw(info.host_api, info.plugin_token) } {
        Ok(host) => host,
        Err(_) => return PluginStatus::InvalidArgument,
    };
    let log = host.log();
    match create_instance(host) {
        Ok(instance) => {
            // SAFETY: The opaque allocation remains alive until the host calls destroy.
            unsafe { out_handle.write(Box::into_raw(Box::new(instance)).cast::<c_void>()) };
            PluginStatus::Ok
        }
        Err(error) => {
            log.write(3, &format!("Rotating CD initialization failed: {error}"));
            PluginStatus::Internal
        }
    }
}

fn create_instance(host: Host) -> Result<Instance, Error> {
    let host_state = host.host_state()?.get()?;
    let events_api = host.events()?;
    let island_state = events_api.island_state()?;
    let has_media = host_state.media_title[0] != 0;
    let cover = if has_media { album_art(&host)? } else { None };
    let surface = Arc::new(host.surfaces()?.create(SurfaceSpecV2 {
        key: winisland_plugin_api::str_to_fixed("rotating-cd-overlay"),
        title: winisland_plugin_api::str_to_fixed(PLUGIN_NAME),
        kind: winisland_plugin_api::SURFACE_FOREGROUND,
        flags: winisland_plugin_api::SURFACE_ENABLED,
        width: MINI_COVER_SIZE,
        height: MINI_COVER_SIZE,
        ..Default::default()
    })?);
    let state = Arc::new(Mutex::new(RenderState {
        cover,
        has_media,
        playing: host_state.is_playing != 0,
        expanded: island_state.expanded != 0,
        angle: 0.0,
    }));

    draw_overlay(&surface, &state)?;
    set_animation(&host, surface.id(), &state)?;

    let state_host = host.clone();
    let state_surface = Arc::clone(&surface);
    let state_render = Arc::clone(&state);
    let events_api = host.events()?;
    let state_events = events_api.subscribe(
        winisland_plugin_api::EVENT_HOST | winisland_plugin_api::EVENT_MEDIA,
        WidgetId::INVALID,
        move |event| {
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Error> {
                let host_state = state_host.host_state()?.get()?;
                let expanded = state_host.events()?.island_state()?.expanded != 0;
                let mut render = state_render
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                render.has_media = host_state.media_title[0] != 0;
                render.playing = host_state.is_playing != 0;
                render.expanded = expanded;
                if should_refresh_cover(event.kind) {
                    render.cover = if render.has_media {
                        album_art(&state_host)?
                    } else {
                        None
                    };
                }
                drop(render);
                set_animation(&state_host, state_surface.id(), &state_render)?;
                draw_overlay(&state_surface, &state_render)
            }));
            log_callback_result(&state_host, "media/state update", result);
        },
    )?;

    let resize_host = host.clone();
    let resize_surface = Arc::clone(&surface);
    let resize_render = Arc::clone(&state);
    let resize_events = events_api.subscribe(
        winisland_plugin_api::EVENT_RESIZE,
        surface.id(),
        move |_| {
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Error> {
                let expanded = resize_host.events()?.island_state()?.expanded != 0;
                resize_render
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .expanded = expanded;
                set_animation(&resize_host, resize_surface.id(), &resize_render)?;
                draw_overlay(&resize_surface, &resize_render)
            }));
            log_callback_result(&resize_host, "resize", result);
        },
    )?;

    Ok(Instance {
        host,
        surface: Some(surface),
        state,
        state_events: Some(state_events),
        resize_events: Some(resize_events),
    })
}

fn log_callback_result(
    host: &Host,
    operation: &str,
    result: Result<Result<(), Error>, Box<dyn std::any::Any + Send>>,
) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            host.log()
                .write(3, &format!("Rotating CD {operation} failed: {error}"));
        }
        Err(_) => {
            host.log().write(
                3,
                &format!("Rotating CD {operation} panicked; update was ignored"),
            );
        }
    }
}

fn album_art(host: &Host) -> Result<Option<ImageHandle>, Error> {
    match host.images()?.album_art() {
        Ok(cover) => Ok(Some(cover)),
        Err(Error::Io) => Ok(None),
        Err(error) => Err(error),
    }
}

fn should_refresh_cover(event_kind: u64) -> bool {
    event_kind & (winisland_plugin_api::EVENT_HOST | winisland_plugin_api::EVENT_MEDIA) != 0
}

fn set_animation(host: &Host, target: WidgetId, state: &Mutex<RenderState>) -> Result<(), Error> {
    let render = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    host.events()?.set_animation(
        target,
        render.has_media && render.playing && !render.expanded,
    )
}

fn draw_overlay(surface: &Surface, state: &Mutex<RenderState>) -> Result<(), Error> {
    let (width, height) = surface.logical_size();
    let disc_x = MINI_COVER_LEFT_INSET + (MINI_COVER_SIZE - DISC_DIAMETER) * 0.5;
    if width <= disc_x + DISC_DIAMETER || height < DISC_DIAMETER {
        return Ok(());
    }
    let state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut frame = DrawListBuilder::new(Size::new(width, height));
    if state.has_media
        && !state.expanded
        && let Some(cover) = state.cover.as_ref()
    {
        draw_disc(
            &mut frame,
            cover,
            disc_x,
            (height - DISC_DIAMETER) * 0.5,
            DISC_DIAMETER,
            state.angle,
        );
    }
    surface.submit(frame.finish())
}

fn draw_disc(
    frame: &mut DrawListBuilder,
    cover: &ImageHandle,
    x: f32,
    y: f32,
    diameter: f32,
    angle: f32,
) {
    let radius = diameter * 0.5;
    let center_x = x + radius;
    let center_y = y + radius;
    let bounds = winisland_plugin_api::sdk::Rect::new(x, y, diameter, diameter);
    let silver = Rgba::from_argb(0xdde8_f2ff);
    let radians = angle.to_radians();
    let (sin, cos) = radians.sin_cos();

    frame.fill_circle(center_x, center_y, radius, Rgba::from_argb(0xff08_0a0d));
    frame.clip_round_rect(bounds, radius);
    frame.transform([
        cos,
        sin,
        -sin,
        cos,
        center_x - cos * center_x + sin * center_y,
        center_y - sin * center_x - cos * center_y,
    ]);

    let image_diameter = diameter * std::f32::consts::SQRT_2;
    let image_bounds = winisland_plugin_api::sdk::Rect::new(
        center_x - image_diameter * 0.5,
        center_y - image_diameter * 0.5,
        image_diameter,
        image_diameter,
    );
    frame.image(
        cover.id(),
        image_bounds,
        winisland_plugin_api::sdk::ImageFit::Cover,
    );

    for groove_radius in [radius * 0.67, radius * 0.84] {
        let groove_bounds = winisland_plugin_api::sdk::Rect::new(
            center_x - groove_radius,
            center_y - groove_radius,
            groove_radius * 2.0,
            groove_radius * 2.0,
        );
        frame.stroke_arc(
            groove_bounds,
            winisland_plugin_api::sdk::Deg12(0.0),
            winisland_plugin_api::sdk::Deg12(360.0),
            0.35,
            Rgba::from_argb(0x30ff_ffff),
        );
    }

    frame.pop_transform();
    frame.pop_clip();

    frame.stroke_arc(
        bounds,
        winisland_plugin_api::sdk::Deg12(0.0),
        winisland_plugin_api::sdk::Deg12(360.0),
        (diameter * 0.035).max(0.65),
        silver,
    );
}

unsafe extern "C" fn on_tick(
    handle: PluginHandleV2,
    target: WidgetId,
    dt_seconds: f64,
) -> PluginStatus {
    if handle.is_null() || !dt_seconds.is_finite() || dt_seconds < 0.0 {
        return PluginStatus::InvalidArgument;
    }
    if dt_seconds == 0.0 {
        return PluginStatus::Ok;
    }
    // SAFETY: The host only calls on_tick for an instance allocated by create.
    let instance = unsafe { &mut *handle.cast::<Instance>() };
    let Some(surface) = instance.surface.as_ref() else {
        return PluginStatus::StaleHandle;
    };
    if target != surface.id() {
        return PluginStatus::Ok;
    }

    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Error> {
        let mut state = instance
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.has_media || !state.playing || state.expanded {
            return Ok(());
        }
        state.angle = advance_angle(state.angle, dt_seconds);
        drop(state);
        draw_overlay(surface, &instance.state)
    }));
    match result {
        Ok(Ok(())) => PluginStatus::Ok,
        Ok(Err(error)) => {
            instance
                .host
                .log()
                .write(3, &format!("Rotating CD frame failed: {error}"));
            PluginStatus::Internal
        }
        Err(_) => {
            instance
                .host
                .log()
                .write(3, "Rotating CD frame panicked; frame was ignored");
            PluginStatus::Internal
        }
    }
}

fn advance_angle(angle: f32, dt_seconds: f64) -> f32 {
    (angle + (dt_seconds.rem_euclid(ROTATION_SECONDS) * 360.0 / ROTATION_SECONDS) as f32)
        .rem_euclid(360.0)
}

unsafe extern "C" fn shutdown(handle: PluginHandleV2) -> PluginStatus {
    if handle.is_null() {
        return PluginStatus::InvalidArgument;
    }
    // SAFETY: The host calls shutdown before destroy for this live instance.
    let instance = unsafe { &mut *handle.cast::<Instance>() };
    for callbacks in [&mut instance.state_events, &mut instance.resize_events] {
        if let Some(callback) = callbacks.as_mut()
            && let Err(error) = callback.cancel()
        {
            instance
                .host
                .log()
                .write(3, &format!("Rotating CD callback shutdown failed: {error}"));
            return PluginStatus::Internal;
        }
    }
    drop(instance.state_events.take());
    drop(instance.resize_events.take());
    drop(instance.surface.take());
    let mut state = instance
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(state.cover.take());
    PluginStatus::Ok
}

unsafe extern "C" fn destroy(handle: PluginHandleV2) {
    if !handle.is_null() {
        // SAFETY: The host calls destroy once after successful shutdown.
        unsafe { drop(Box::from_raw(handle.cast::<Instance>())) };
    }
}

/// # Safety
/// WinIsland calls this exported function using the ABI v2 entry signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn winisland_plugin_entry_v2() -> *const PluginDescriptorV2 {
    &DESCRIPTOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_uses_abi_v2_and_does_not_replace_native_music_activity() {
        assert_eq!(DESCRIPTOR.abi_version, ABI_VERSION_2);
        assert_eq!(
            &DESCRIPTOR.metadata.id[..PLUGIN_ID.len()],
            PLUGIN_ID.as_bytes()
        );
        assert_eq!(DESCRIPTOR.capabilities & CAP_EVENTS, CAP_EVENTS);
        assert_eq!(DESCRIPTOR.capabilities & CAP_IMAGE, CAP_IMAGE);
        assert_eq!(DESCRIPTOR.capabilities & CAP_SURFACE, CAP_SURFACE);
        assert_eq!(
            DESCRIPTOR.capabilities & winisland_plugin_api::abi::CAP_ACTIVITY,
            0
        );
    }

    #[test]
    fn disc_rotation_angle_wraps_cleanly() {
        assert_eq!(advance_angle(359.0, 2.0), 287.0);
        assert_eq!(advance_angle(0.0, ROTATION_SECONDS), 0.0);
        assert_eq!(advance_angle(0.0, 0.5), 72.0);
    }

    #[test]
    fn album_art_refreshes_for_host_and_media_events() {
        assert!(should_refresh_cover(winisland_plugin_api::EVENT_HOST));
        assert!(should_refresh_cover(winisland_plugin_api::EVENT_MEDIA));
        assert!(!should_refresh_cover(winisland_plugin_api::EVENT_RESIZE));
    }
}
