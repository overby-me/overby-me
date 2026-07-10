use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlImageElement, WebGlRenderingContext as GL, WebGlTexture};

use manganis::asset;

fn icon_url(icon: &str) -> String {
    match icon {
        "me.avif" => asset!("/assets/icons/me.avif").to_string(),
        "commerce.avif" => asset!("/assets/icons/commerce.avif").to_string(),
        "improve.avif" => asset!("/assets/icons/improve.avif").to_string(),
        "connect.avif" => asset!("/assets/icons/connect.avif").to_string(),
        "immerse.avif" => asset!("/assets/icons/immerse.avif").to_string(),
        "give.avif" => asset!("/assets/icons/give.avif").to_string(),
        "fediverse.avif" => asset!("/assets/icons/fediverse.avif").to_string(),
        "linkedin.avif" => asset!("/assets/icons/linkedin.avif").to_string(),
        "pinkleap.avif" => asset!("/assets/icons/pinkleap.avif").to_string(),
        "mail.avif" => asset!("/assets/icons/mail.avif").to_string(),
        "matrix.avif" => asset!("/assets/icons/matrix.avif").to_string(),
        "signal.avif" => asset!("/assets/icons/signal.avif").to_string(),
        "rocksky.avif" => asset!("/assets/icons/rocksky.avif").to_string(),
        "atmosphere.avif" => asset!("/assets/icons/atmosphere.avif").to_string(),
        "bridgy.avif" => asset!("/assets/icons/bridgy.avif").to_string(),
        "github.avif" => asset!("/assets/icons/github.avif").to_string(),
        "codeberg.avif" => asset!("/assets/icons/codeberg.avif").to_string(),
        "tangled.avif" => asset!("/assets/icons/tangled.avif").to_string(),
        "mastodon.avif" => asset!("/assets/icons/mastodon.avif").to_string(),
        "bluesky.avif" => asset!("/assets/icons/bluesky.avif").to_string(),
        "radikale.avif" => asset!("/assets/icons/radikale.avif").to_string(),
        "aivero.avif" => asset!("/assets/icons/aivero.avif").to_string(),
        "factbird.avif" => asset!("/assets/icons/factbird.avif").to_string(),
        "veo.avif" => asset!("/assets/icons/veo.avif").to_string(),
        "wikipedia.avif" => asset!("/assets/icons/wikipedia.avif").to_string(),
        "happycow.avif" => asset!("/assets/icons/happycow.avif").to_string(),
        "lemmy.avif" => asset!("/assets/icons/lemmy.avif").to_string(),
        "neodb.avif" => asset!("/assets/icons/neodb.avif").to_string(),
        other => {
            log::warn!("Unknown icon: {other}");
            String::new()
        }
    }
}

pub struct TextureManager {
    gl: GL,
    pub textures: Rc<RefCell<HashMap<String, WebGlTexture>>>,
}

impl TextureManager {
    pub fn new(gl: GL) -> Self {
        Self {
            gl,
            textures: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    pub fn load_icon(&self, icon: &str) {
        let gl = self.gl.clone();
        let textures = Rc::clone(&self.textures);
        let icon_name = icon.to_string();
        let src = icon_url(icon);

        if src.is_empty() {
            return;
        }

        // Create a placeholder texture immediately
        if let Some(tex) = gl.create_texture() {
            gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
            // 1x1 transparent pixel as placeholder
            let pixel: [u8; 4] = [0, 0, 0, 0];
            let _ = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                GL::TEXTURE_2D,
                0,
                GL::RGBA as i32,
                1,
                1,
                0,
                GL::RGBA,
                GL::UNSIGNED_BYTE,
                Some(&pixel),
            );
            textures.borrow_mut().insert(icon_name.clone(), tex);
        }

        let Ok(img) = HtmlImageElement::new() else {
            // The placeholder texture is already in place; skip the async load.
            return;
        };
        img.set_cross_origin(Some("anonymous"));

        let gl_clone = gl.clone();
        let textures_clone = Rc::clone(&textures);
        let icon_clone = icon_name.clone();
        let img_clone = img.clone();

        let onload = Closure::wrap(Box::new(move || {
            if let Some(tex) = gl_clone.create_texture() {
                gl_clone.bind_texture(GL::TEXTURE_2D, Some(&tex));
                gl_clone.pixel_storei(GL::UNPACK_FLIP_Y_WEBGL, 1);
                let _ = gl_clone.tex_image_2d_with_u32_and_u32_and_image(
                    GL::TEXTURE_2D,
                    0,
                    GL::RGBA as i32,
                    GL::RGBA,
                    GL::UNSIGNED_BYTE,
                    &img_clone,
                );
                gl_clone.tex_parameteri(
                    GL::TEXTURE_2D,
                    GL::TEXTURE_WRAP_S,
                    GL::CLAMP_TO_EDGE as i32,
                );
                gl_clone.tex_parameteri(
                    GL::TEXTURE_2D,
                    GL::TEXTURE_WRAP_T,
                    GL::CLAMP_TO_EDGE as i32,
                );
                gl_clone.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::LINEAR as i32);
                gl_clone.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
                textures_clone.borrow_mut().insert(icon_clone.clone(), tex);
            }
        }) as Box<dyn FnMut()>);

        img.set_onload(Some(onload.as_ref().unchecked_ref()));
        onload.forget(); // Leak the closure — it only fires once

        img.set_src(&src);
    }
}
