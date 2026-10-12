use anyhow::anyhow;
use cpal::traits::{DeviceTrait, HostTrait};

/// `hint` が空なら既定の入力デバイス、そうでなければ名前に `hint` を含むもの（大文字小文字は区別しない）。
pub(crate) fn input(hint: &str) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    if hint.is_empty() {
        return host
            .default_input_device()
            .ok_or_else(|| anyhow!("既定の入力デバイスがありません"));
    }
    find(host.input_devices()?, hint)
        .ok_or_else(|| anyhow!("名前に「{hint}」を含む入力デバイスがありません"))
}

/// `hint` が空なら既定の出力デバイス、そうでなければ名前に `hint` を含むもの。
pub(crate) fn output(hint: &str) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    if hint.is_empty() {
        return host
            .default_output_device()
            .ok_or_else(|| anyhow!("既定の出力デバイスがありません"));
    }
    find(host.output_devices()?, hint)
        .ok_or_else(|| anyhow!("名前に「{hint}」を含む出力デバイスがありません"))
}

fn find(mut devices: impl Iterator<Item = cpal::Device>, hint: &str) -> Option<cpal::Device> {
    let hint = hint.to_lowercase();
    devices.find(|d| name(d).is_some_and(|n| n.to_lowercase().contains(&hint)))
}

fn name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.to_string())
}

/// 入力デバイスの名前の一覧。既定のものには「(既定)」を付ける。
pub fn list_inputs() -> anyhow::Result<Vec<String>> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| name(&d));
    Ok(mark_default(
        host.input_devices()?.filter_map(|d| name(&d)),
        default,
    ))
}

/// 出力デバイスの名前の一覧。既定のものには「(既定)」を付ける。
pub fn list_outputs() -> anyhow::Result<Vec<String>> {
    let host = cpal::default_host();
    let default = host.default_output_device().and_then(|d| name(&d));
    Ok(mark_default(
        host.output_devices()?.filter_map(|d| name(&d)),
        default,
    ))
}

fn mark_default(names: impl Iterator<Item = String>, default: Option<String>) -> Vec<String> {
    names
        .map(|n| {
            if default.as_deref() == Some(n.as_str()) {
                format!("{n}  (既定)")
            } else {
                n
            }
        })
        .collect()
}
