use anyhow::{bail, Result};

use crate::config::{MappingConfig, MappingMode};

#[derive(Clone)]
pub struct VelocityMapper {
    table: [u8; 128],
    bypass: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingReport {
    Mapped {
        channel: u8,
        note: u8,
        input: u8,
        output: u8,
    },
    Passthrough,
}

impl VelocityMapper {
    pub fn bypass() -> Self {
        Self {
            table: identity_table(),
            bypass: true,
        }
    }

    pub fn from_config(config: Option<&MappingConfig>) -> Result<Self> {
        let table = match config {
            Some(config) => match config.mode {
                MappingMode::Curve => curve_table(
                    config.gamma.unwrap_or(1.0),
                    config.min_out.unwrap_or(1),
                    config.max_out.unwrap_or(127),
                )?,
                MappingMode::Table => table_from_config(config.velocity_table.as_deref())?,
                MappingMode::Piecewise => piecewise_table(config.points.as_deref())?,
            },
            None => curve_table(1.0, 1, 127)?,
        };

        Ok(Self {
            table,
            bypass: false,
        })
    }

    pub fn map_message(&self, message: &[u8]) -> (Vec<u8>, MappingReport) {
        let mut out = message.to_vec();

        if self.bypass || out.len() < 3 {
            return (out, MappingReport::Passthrough);
        }

        let status = out[0];
        let message_type = status & 0xF0;
        let channel = status & 0x0F;

        if message_type == 0x90 && out[2] > 0 {
            let input = out[2];
            let mapped = self.table[input as usize];
            out[2] = mapped;
            return (
                out,
                MappingReport::Mapped {
                    channel: channel + 1,
                    note: message[1],
                    input,
                    output: mapped,
                },
            );
        }

        (out, MappingReport::Passthrough)
    }
}

fn identity_table() -> [u8; 128] {
    let mut table = [0; 128];
    for (i, value) in table.iter_mut().enumerate() {
        *value = i as u8;
    }
    table
}

fn curve_table(gamma: f32, min_out: u8, max_out: u8) -> Result<[u8; 128]> {
    if gamma <= 0.0 || !gamma.is_finite() {
        bail!("mapping.gamma must be a positive finite number");
    }
    if min_out == 0 || min_out > 127 || max_out == 0 || max_out > 127 || min_out > max_out {
        bail!("mapping.min_out and mapping.max_out must be 1..=127 and min_out <= max_out");
    }

    let mut table = [0; 128];
    table[0] = 0;
    let span = (max_out - min_out) as f32;

    for velocity in 1..=127 {
        let normalized = velocity as f32 / 127.0;
        let shaped = normalized.powf(gamma);
        let mapped = min_out as f32 + shaped * span;
        table[velocity] = mapped.round().clamp(min_out as f32, max_out as f32) as u8;
    }

    Ok(table)
}

fn table_from_config(values: Option<&[u8]>) -> Result<[u8; 128]> {
    let values = values
        .ok_or_else(|| anyhow::anyhow!("mapping.velocity_table is required for table mode"))?;
    if values.len() != 128 {
        bail!("mapping.velocity_table must contain exactly 128 values");
    }
    if values.iter().any(|value| *value > 127) {
        bail!("mapping.velocity_table values must be in 0..=127");
    }

    let mut table = [0; 128];
    table.copy_from_slice(values);
    table[0] = 0;
    Ok(table)
}

fn piecewise_table(points: Option<&[[u8; 2]]>) -> Result<[u8; 128]> {
    let points =
        points.ok_or_else(|| anyhow::anyhow!("mapping.points is required for piecewise mode"))?;
    if points.len() < 2 {
        bail!("mapping.points must contain at least two points");
    }
    if points[0][0] != 0 || points[0][1] != 0 {
        bail!("mapping.points must start with [0, 0]");
    }
    if points[points.len() - 1][0] != 127 {
        bail!("mapping.points must end at input velocity 127");
    }

    for pair in points.windows(2) {
        if pair[0][0] >= pair[1][0] {
            bail!("mapping.points input velocities must be strictly increasing");
        }
    }

    let mut table = [0; 128];

    for pair in points.windows(2) {
        let [x0, y0] = pair[0];
        let [x1, y1] = pair[1];
        let dx = (x1 - x0) as f32;

        for x in x0..=x1 {
            let t = (x - x0) as f32 / dx;
            let y = y0 as f32 + t * (y1 as f32 - y0 as f32);
            table[x as usize] = y.round().clamp(0.0, 127.0) as u8;
        }
    }

    table[0] = 0;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_velocity_zero_passes_through() {
        let mapper = VelocityMapper::from_config(None).unwrap();
        let (out, report) = mapper.map_message(&[0x90, 60, 0]);
        assert_eq!(out, vec![0x90, 60, 0]);
        assert_eq!(report, MappingReport::Passthrough);
    }

    #[test]
    fn note_off_passes_through() {
        let mapper = VelocityMapper::from_config(None).unwrap();
        let (out, report) = mapper.map_message(&[0x80, 60, 64]);
        assert_eq!(out, vec![0x80, 60, 64]);
        assert_eq!(report, MappingReport::Passthrough);
    }

    #[test]
    fn note_on_maps_velocity() {
        let mapper = VelocityMapper {
            table: {
                let mut table = identity_table();
                table[40] = 50;
                table
            },
            bypass: false,
        };
        let (out, report) = mapper.map_message(&[0x91, 60, 40]);
        assert_eq!(out, vec![0x91, 60, 50]);
        assert_eq!(
            report,
            MappingReport::Mapped {
                channel: 2,
                note: 60,
                input: 40,
                output: 50
            }
        );
    }

    #[test]
    fn piecewise_mapping_interpolates_between_points() {
        let table = piecewise_table(Some(&[
            [0, 0],
            [1, 0],
            [30, 22],
            [60, 66],
            [85, 93],
            [117, 127],
            [127, 127],
        ]))
        .unwrap();
        assert_eq!(table[0], 0);
        assert_eq!(table[1], 0);
        assert_eq!(table[30], 22);
        assert_eq!(table[60], 66);
        assert_eq!(table[85], 93);
        assert_eq!(table[117], 127);
        assert_eq!(table[127], 127);
        assert_eq!(table[45], 44);
    }

    #[test]
    fn linear_top_fix_preserves_the_low_end_and_reaches_full_velocity() {
        let table = piecewise_table(Some(&[[0, 0], [1, 0], [117, 127], [127, 127]])).unwrap();

        assert_eq!(table[1], 0);
        assert_eq!(table[30], 32);
        assert_eq!(table[60], 65);
        assert_eq!(table[100], 108);
        assert_eq!(table[117], 127);
        assert_eq!(table[127], 127);
    }

    #[test]
    fn clp_curve_matches_its_control_points() {
        let points = [
            [0, 0],
            [1, 0],
            [6, 3],
            [7, 4],
            [15, 14],
            [30, 48],
            [60, 91],
            [90, 117],
            [120, 127],
            [127, 127],
        ];
        let table = piecewise_table(Some(&points)).unwrap();

        for [input, output] in points {
            assert_eq!(table[input as usize], output);
        }
    }
}
