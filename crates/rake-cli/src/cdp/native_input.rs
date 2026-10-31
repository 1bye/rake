//! Connection-owned native window selection and literal CDP input translation.

use super::{TIMEOUT, wire::Error};
use rake_client::local::Instance;
use rake_protocol::{
    control::{Command, Query, Reply},
    inspection::preview::{Input, Point, PointerPhase, Window as NativeWindow},
    router::WindowId,
};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct NativeInput {
    pub window: Option<WindowId>,
    pointer: [f64; 2],
}

pub(super) fn windows(instance: &Instance) -> Result<Vec<NativeWindow>, Error> {
    let Reply::Data(value) = crate::terminal::query(instance, Query::Preview {}, TIMEOUT)
        .map_err(|e| Error::new(-32000, e.to_string()))?
    else {
        return Err(Error::new(-32000, "Unexpected native window reply"));
    };
    serde_json::from_value(value).map_err(|e| Error::new(-32000, e.to_string()))
}

impl NativeInput {
    pub fn target(&self, instance: &Instance) -> Result<NativeWindow, Error> {
        self.choose(windows(instance)?)
    }

    fn choose(&self, mut available: Vec<NativeWindow>) -> Result<NativeWindow, Error> {
        if let Some(id) = self.window {
            return available
                .into_iter()
                .find(|w| w.window == id)
                .ok_or_else(|| Error::new(-32000, "Selected native window is unavailable"));
        }
        if available.len() != 1 {
            return Err(Error::new(
                -32000,
                "Select a native window with Rake.selectWindow; automatic selection requires exactly one window",
            ));
        }
        Ok(available.remove(0))
    }

    pub fn select(&mut self, instance: &Instance, params: &Value) -> Result<Value, Error> {
        let id = params["windowId"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| Error::new(-32602, "Positive windowId required"))?;
        let available = windows(instance)?;
        let window = available
            .into_iter()
            .find(|w| w.window == WindowId(id))
            .ok_or_else(|| Error::new(-32000, "Native window is unavailable"))?;
        self.window = Some(window.window);
        self.pointer = [0., 0.];
        Ok(json!({"window":window}))
    }

    pub fn command(
        &mut self,
        instance: &Instance,
        method: &str,
        params: &Value,
    ) -> Result<Command, Error> {
        let (position, input) = translate(method, params, self.pointer)?;
        let target = self.target(instance)?;
        if position[0] < 0.
            || position[1] < 0.
            || position[0] >= f64::from(target.content.width)
            || position[1] >= f64::from(target.content.height)
        {
            return Err(Error::new(
                -32602,
                "Input coordinates are outside the selected content viewport",
            ));
        }
        let geometry = [
            target.frame.width,
            target.frame.height,
            target.content.x,
            target.content.y,
            target.content.width,
            target.content.height,
        ]
        .map(|n| (n * 64.).round() as i32);
        self.pointer = position;
        Ok(Command::PreviewInput {
            point: Point {
                window: target.window,
                native_id: target.native_id,
                generation: target.generation,
                geometry: Some(geometry),
                x: position[0].round() as i32,
                y: position[1].round() as i32,
            },
            input,
        })
    }
}

fn bounded_u8(params: &Value, name: &str, default: u8, max: u8) -> Result<u8, Error> {
    match params.get(name) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= u64::from(max))
            .map(|n| n as u8)
            .ok_or_else(|| Error::new(-32602, format!("Invalid {name}"))),
    }
}

fn translate(method: &str, params: &Value, pointer: [f64; 2]) -> Result<([f64; 2], Input), Error> {
    let modifiers = bounded_u8(params, "modifiers", 0, 15)?;
    let text = || {
        params["text"]
            .as_str()
            .filter(|s| s.len() <= 16 * 1024)
            .ok_or_else(|| Error::new(-32602, "text must be at most 16 KiB"))
    };
    match method {
        "Input.insertText" => Ok((
            pointer,
            Input::Text {
                text: text()?.into(),
            },
        )),
        "Input.dispatchKeyEvent" => {
            if params["type"] == "char" {
                return Ok((
                    pointer,
                    Input::Text {
                        text: text()?.into(),
                    },
                ));
            }
            let down = match params["type"].as_str() {
                Some("keyDown" | "rawKeyDown") => true,
                Some("keyUp") => false,
                _ => return Err(Error::new(-32602, "Unsupported key event type")),
            };
            let key = params["key"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .ok_or_else(|| Error::new(-32602, "key must contain 1–128 bytes"))?;
            let key = match key {
                "ArrowLeft" => "left",
                "ArrowRight" => "right",
                "ArrowUp" => "up",
                "ArrowDown" => "down",
                "Enter" => "enter",
                "Escape" => "escape",
                "Backspace" => "backspace",
                "Delete" => "delete",
                "Tab" => "tab",
                " " => "space",
                other => other,
            };
            let text = if down && params["type"] != "rawKeyDown" && params.get("text").is_some() {
                Some(text()?.into())
            } else {
                None
            };
            Ok((
                pointer,
                Input::Key {
                    down,
                    key: key.into(),
                    text,
                    modifiers,
                },
            ))
        }
        "Input.dispatchMouseEvent" => {
            let coordinate = |name: &str| {
                params[name]
                    .as_f64()
                    .filter(|n| n.is_finite() && n.abs() < 1_000_000.)
                    .ok_or_else(|| Error::new(-32602, format!("Finite {name} required")))
            };
            let position = [coordinate("x")?, coordinate("y")?];
            if params["type"] == "mouseWheel" {
                if modifiers != 0 {
                    return Err(Error::new(
                        -32602,
                        "Modified wheel gestures are unavailable",
                    ));
                }
                let delta = |name: &str| {
                    params[name]
                        .as_f64()
                        .filter(|n| n.is_finite() && n.abs() <= 2000.)
                        .map(|n| (n * 64.).round() as i32)
                        .ok_or_else(|| Error::new(-32602, format!("Invalid {name}")))
                };
                return Ok((
                    position,
                    Input::Wheel {
                        dx: delta("deltaX")?,
                        dy: delta("deltaY")?,
                        phase: rake_protocol::inspection::preview::WheelPhase::Moved,
                    },
                ));
            }
            let phase = match params["type"].as_str() {
                Some("mousePressed") => PointerPhase::Down,
                Some("mouseReleased") => PointerPhase::Up,
                Some("mouseMoved") => PointerPhase::Move,
                _ => return Err(Error::new(-32602, "Unsupported pointer phase")),
            };
            let button = match params["button"].as_str() {
                None | Some("none") => 0,
                Some("left") => 1,
                Some("right") => 2,
                Some("middle") => 4,
                _ => return Err(Error::new(-32602, "Unsupported mouse button")),
            };
            let button = if phase == PointerPhase::Move {
                let buttons = bounded_u8(params, "buttons", button, 4)?;
                if !matches!(buttons, 0 | 1 | 2 | 4) {
                    return Err(Error::new(
                        -32602,
                        "Multiple pressed buttons are unavailable",
                    ));
                }
                buttons
            } else {
                button
            };
            if phase != PointerPhase::Move && button == 0 {
                return Err(Error::new(-32602, "A button is required for press/release"));
            }
            let click_count = bounded_u8(params, "clickCount", 1, 3)?.max(1);
            Ok((
                position,
                Input::Pointer {
                    phase,
                    button,
                    click_count,
                    modifiers,
                },
            ))
        }
        _ => Err(Error::new(-32601, "Unsupported native input method")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_never_falls_through_to_another_window() {
        let window: NativeWindow=serde_json::from_value(json!({"window":1,"native_id":9,"generation":"one","frame":{"x":0,"y":0,"width":100,"height":100},"content":{"x":0,"y":0,"width":100,"height":100},"translucent":false})).unwrap();
        let mut input = NativeInput::default();
        assert!(input.choose(vec![]).is_err());
        assert!(input.choose(vec![window.clone(), window.clone()]).is_err());
        assert_eq!(input.choose(vec![window.clone()]).unwrap().native_id, 9);
        input.window = Some(WindowId(2));
        assert!(input.choose(vec![window]).is_err());
    }
    #[test]
    fn literal_input_validates_without_runtime_io_and_preserves_drag_and_unicode() {
        let (_, input) = translate(
            "Input.dispatchMouseEvent",
            &json!({"type":"mouseMoved","x":1.5,"y":2,"buttons":1,"modifiers":8}),
            [0., 0.],
        )
        .unwrap();
        assert!(matches!(
            input,
            Input::Pointer {
                phase: PointerPhase::Move,
                button: 1,
                modifiers: 8,
                ..
            }
        ));
        let (_, input) =
            translate("Input.insertText", &json!({"text":"café 🙂"}), [1., 2.]).unwrap();
        assert_eq!(
            input,
            Input::Text {
                text: "café 🙂".into()
            }
        );
        for params in [
            json!({"type":"mousePressed","x":-1,"y":"bad"}),
            json!({"type":"mouseMoved","x":1,"y":2,"buttons":3}),
            json!({"type":"mouseWheel","x":1,"y":2,"deltaX":0,"deltaY":2001}),
        ] {
            assert!(translate("Input.dispatchMouseEvent", &params, [0., 0.]).is_err());
        }
        assert!(
            translate(
                "Input.insertText",
                &json!({"text":"🙂".repeat(4097)}),
                [0., 0.]
            )
            .is_err()
        );
        let (_, input) = translate(
            "Input.dispatchKeyEvent",
            &json!({"type":"rawKeyDown","key":"ArrowLeft","text":"ignored"}),
            [0., 0.],
        )
        .unwrap();
        assert!(matches!(input,Input::Key {key,text:None,..} if key=="left"));
    }
}
