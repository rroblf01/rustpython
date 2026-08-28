use crate::object::*;
use std::collections::HashMap;

pub fn create_wave_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    fn read_wave_params(data: &[u8]) -> Result<(i32, i32, i32, i32, usize), String> {
        if data.len() < 44 {
            return Err("Not a valid WAV file: too short".to_string());
        }
        if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err("Not a valid WAV file: missing RIFF/WAVE header".to_string());
        }
        // Find fmt chunk — skip RIFF header (12 bytes)
        let mut offset = 12usize;
        let (fmt_offset, fmt_size) = loop {
            if offset + 8 > data.len() {
                return Err("Not a valid WAV file: no fmt chunk found".to_string());
            }
            let chunk_id = &data[offset..offset + 4];
            let chunk_size = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]) as usize;
            if chunk_id == b"fmt " {
                break (offset, chunk_size);
            }
            offset += 8 + chunk_size;
            if offset % 2 != 0 {
                offset += 1;
            } // pad to word boundary
            if offset >= data.len() {
                return Err("Not a valid WAV file: no fmt chunk found".to_string());
            }
        };

        let fmt_data = &data[fmt_offset..];
        if fmt_data.len() < 24 {
            return Err("Not a valid WAV file: fmt chunk too small".to_string());
        }

        let audio_format = u16::from_le_bytes([fmt_data[8], fmt_data[9]]);
        if audio_format != 1 {
            return Err(format!(
                "Unsupported WAV audio format: {} (only PCM/1 supported)",
                audio_format
            ));
        }
        let nchannels = u16::from_le_bytes([fmt_data[10], fmt_data[11]]) as i32;
        let framerate =
            i32::from_le_bytes([fmt_data[12], fmt_data[13], fmt_data[14], fmt_data[15]]);
        // Byte rate is at [16..20], block align at [20..22]
        let bits_per_sample = u16::from_le_bytes([fmt_data[22], fmt_data[23]]);
        let sampwidth = (bits_per_sample / 8) as i32;
        if sampwidth == 0 {
            return Err("Invalid sample width: 0 bytes per sample".to_string());
        }

        // Find data chunk
        let mut data_offset = fmt_offset + 8 + fmt_size;
        if data_offset % 2 != 0 {
            data_offset += 1;
        }

        let (data_chunk_start, data_size) = loop {
            if data_offset + 8 > data.len() {
                return Err("Not a valid WAV file: no data chunk found".to_string());
            }
            let chunk_id = &data[data_offset..data_offset + 4];
            let chunk_size = u32::from_le_bytes([
                data[data_offset + 4],
                data[data_offset + 5],
                data[data_offset + 6],
                data[data_offset + 7],
            ]) as usize;
            if chunk_id == b"data" {
                break (data_offset + 8, chunk_size);
            }
            data_offset += 8 + chunk_size;
            if data_offset % 2 != 0 {
                data_offset += 1;
            }
            if data_offset >= data.len() {
                return Err("Not a valid WAV file: no data chunk found".to_string());
            }
        };

        let nframes = if sampwidth > 0 && nchannels > 0 {
            (data_size as i32) / (sampwidth * nchannels)
        } else {
            0
        };

        Ok((nchannels, sampwidth, framerate, nframes, data_chunk_start))
    }

    // Wave_read module-level alias — direct instantiation not allowed
    d.insert_str(
        "Wave_read",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Wave_read".to_string(),
            func: |_args| {
                Err(PyError::type_error(
                    "Wave_read cannot be instantiated directly; use wave.open()",
                ))
            },
        }),
    );

    d.insert_str(
        "open",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "open".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "open() missing required argument: file",
                    ));
                }
                let file_path = args[0].str();
                let mode = if args.len() > 1 {
                    args[1].str()
                } else {
                    "r".to_string()
                };
                let mode = mode.trim();
                if mode != "r" && mode != "rb" {
                    return Err(PyError::type_error(format!(
                        "wave.open() only supports mode='r' or 'rb', got '{}'",
                        mode
                    )));
                }

                let data = match std::fs::read(&file_path) {
                    Ok(d) => d,
                    Err(e) => {
                        return Err(PyError::type_error(format!("Cannot open wave file: {}", e)))
                    }
                };

                match read_wave_params(&data) {
                    Ok((nchannels, sampwidth, framerate, nframes, data_start)) => {
                        // Build a proper Type with methods so args[0] is self
                        let mut type_dict = HashMap::new();

                        type_dict.insert_str(
                            "getparams",
                            PyObjectRef::new(PyObject::BuiltinFunction {
                                name: "getparams".to_string(),
                                func: |gp_args| {
                                    if gp_args.is_empty() {
                                        return Err(PyError::type_error(
                                            "getparams() missing self argument",
                                        ));
                                    }
                                    let inst = gp_args[0].borrow();
                                    if let PyObject::Instance { dict, .. } = &*inst {
                                        let nc = dict
                                            .get_str("nchannels")
                                            .and_then(|v| v.as_i64())
                                            .unwrap_or(0);
                                        let sw = dict
                                            .get_str("sampwidth")
                                            .and_then(|v| v.as_i64())
                                            .unwrap_or(0);
                                        let fr = dict
                                            .get_str("framerate")
                                            .and_then(|v| v.as_i64())
                                            .unwrap_or(0);
                                        let nf = dict
                                            .get_str("nframes")
                                            .and_then(|v| v.as_i64())
                                            .unwrap_or(0);
                                        Ok(py_tuple(vec![
                                            py_int(nc),
                                            py_int(sw),
                                            py_int(fr),
                                            py_int(nf),
                                            py_str("NONE"),
                                            py_str("not compressed"),
                                        ]))
                                    } else {
                                        Err(PyError::type_error(
                                            "getparams: not a Wave_read instance",
                                        ))
                                    }
                                },
                            }),
                        );

                        type_dict.insert_str(
                            "readframes",
                            PyObjectRef::new(PyObject::BuiltinFunction {
                                name: "readframes".to_string(),
                                func: |rf_args| {
                                    if rf_args.is_empty() {
                                        return Err(PyError::type_error(
                                            "readframes() missing required argument: self",
                                        ));
                                    }
                                    let n = if rf_args.len() > 1 {
                                        rf_args[1].as_i64().ok_or_else(|| {
                                            PyError::type_error(
                                                "readframes() argument must be an integer",
                                            )
                                        })? as usize
                                    } else {
                                        0
                                    };
                                    if n == 0 {
                                        return Ok(PyObjectRef::imm(PyObject::Bytes(vec![])));
                                    }
                                    // Read nchannels, sampwidth, _data, _data_start from instance dict
                                    let (nc_r, sw_r, dc_opt, ds_r) = {
                                        let inst = rf_args[0].borrow();
                                        if let PyObject::Instance { dict, .. } = &*inst {
                                            let nc_r = dict
                                                .get_str("nchannels")
                                                .and_then(|v| v.as_i64())
                                                .unwrap_or(0)
                                                as usize;
                                            let sw_r = dict
                                                .get_str("sampwidth")
                                                .and_then(|v| v.as_i64())
                                                .unwrap_or(0)
                                                as usize;
                                            let dc_opt = dict.get_str("_data").cloned();
                                            let ds_r = dict
                                                .get_str("_data_start")
                                                .and_then(|v| v.as_i64())
                                                .unwrap_or(0)
                                                as usize;
                                            (nc_r, sw_r, dc_opt, ds_r)
                                        } else {
                                            return Err(PyError::type_error(
                                                "readframes: not a Wave_read instance",
                                            ));
                                        }
                                    };
                                    let frame_size = sw_r * nc_r;
                                    if frame_size == 0 {
                                        return Ok(PyObjectRef::imm(PyObject::Bytes(vec![])));
                                    }
                                    let dc = match dc_opt {
                                        Some(d) => {
                                            let b = d.borrow();
                                            if let PyObject::Bytes(byte_data) = &*b {
                                                byte_data.clone()
                                            } else {
                                                vec![]
                                            }
                                        }
                                        None => vec![],
                                    };
                                    let nframes_avail = dc.len().saturating_sub(ds_r) / frame_size;
                                    let n_to_read = n.min(nframes_avail);
                                    let end = ds_r + n_to_read * frame_size;
                                    if end > dc.len() || end <= ds_r {
                                        return Ok(PyObjectRef::imm(PyObject::Bytes(vec![])));
                                    }
                                    let frame_data = dc[ds_r..end].to_vec();
                                    Ok(PyObjectRef::imm(PyObject::Bytes(frame_data)))
                                },
                            }),
                        );

                        type_dict.insert_str(
                            "close",
                            PyObjectRef::new(PyObject::BuiltinFunction {
                                name: "close".to_string(),
                                func: |_| Ok(py_none()),
                            }),
                        );

                        let typ = PyObjectRef::new(PyObject::Type {
                            name: "Wave_read".to_string(),
                            dict: Box::new(str_map_to_typedict(type_dict)),
                            bases: vec![],
                            mro: vec![],
                        });

                        let mut instance_dict = AttrMap::new();
                        instance_dict.insert_str("nchannels", py_int(nchannels as i64));
                        instance_dict.insert_str("sampwidth", py_int(sampwidth as i64));
                        instance_dict.insert_str("framerate", py_int(framerate as i64));
                        instance_dict.insert_str("nframes", py_int(nframes as i64));
                        instance_dict.insert_str("comptype", py_str("NONE"));
                        instance_dict.insert_str("compname", py_str("not compressed"));
                        instance_dict
                            .insert_str("_data", PyObjectRef::imm(PyObject::Bytes(data.clone())));
                        instance_dict.insert_str("_data_start", py_int(data_start as i64));

                        Ok(PyObjectRef::new(PyObject::Instance {
                            typ,
                            dict: instance_dict,
                        }))
                    }
                    Err(e) => Err(PyError::type_error(e)),
                }
            },
        }),
    );

    // wave._byteswap — byte-swap helper for multi-byte samples.
    // CPython's Lib/wave.py defines this; some code imports it directly.
    d.insert(
        "_byteswap".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_byteswap".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "_byteswap() missing 2 required positional arguments: 'data' and 'width'",
                    ));
                }
                let data_bytes = args[0].borrow();
                let data = match &*data_bytes {
                    PyObject::Bytes(b) => b.clone(),
                    _ => {
                        return Err(PyError::type_error(
                            "_byteswap() argument 'data' must be bytes",
                        ))
                    }
                };
                let width = args[1].as_i64().ok_or_else(|| {
                    PyError::type_error("_byteswap() argument 'width' must be an int")
                })? as usize;
                if width < 1 || width > 8 {
                    return Err(PyError::type_error(
                        "_byteswap() argument 'width' must be between 1 and 8",
                    ));
                }
                // Reverse each sample of `width` bytes
                let mut out = Vec::with_capacity(data.len());
                for chunk in data.chunks(width) {
                    let mut sample = chunk.to_vec();
                    sample.reverse();
                    out.extend_from_slice(&sample);
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(out)))
            },
        }),
    );

    d
}
