use crate::config::McConfig;
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};
use wat::parse_str;

const DEFAULT_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "validate") (param i32 i32) (result i32)
    (if (result i32) (i32.eqz (local.get 1))
      (then (i32.const 0))
      (else (i32.const 1))
    )
  )
)
"#;

pub struct AgentBooster {
    module: Option<Module>,
    engine: Engine,
    allow_short_circuit: bool,
}

impl AgentBooster {
    pub fn load(config: &McConfig) -> Result<Self> {
        let engine = Engine::default();
        let module = if config.booster_enabled {
            let wasm_bytes = if let Some(path) = &config.booster_wasm {
                fs::read(path)
                    .with_context(|| format!("failed to read booster wasm at {path:?}"))?
            } else {
                parse_str(DEFAULT_WAT).context("failed to compile default booster wasm")?
            };
            Some(Module::new(&engine, wasm_bytes).context("failed to compile booster module")?)
        } else {
            None
        };
        Ok(Self {
            module,
            engine,
            allow_short_circuit: config.booster_allow_short_circuit,
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.module.is_some()
    }

    pub fn allow_short_circuit(&self) -> bool {
        self.allow_short_circuit
    }

    pub fn run(&self, payload: &Value) -> Result<bool> {
        if let Some(module) = &self.module {
            let mut store = Store::new(&self.engine, ());
            let instance = Instance::new(&mut store, module, &[])
                .context("failed to instantiate booster module")?;
            let memory = instance
                .get_memory(&mut store, "memory")
                .context("booster wasm missing memory export")?;
            let validate: TypedFunc<(i32, i32), i32> = instance
                .get_typed_func(&mut store, "validate")
                .context("booster wasm missing validate export")?;
            let json = serde_json::to_string(payload)
                .context("failed to serialize payload for booster")?;
            let len = json.len() as i32;
            ensure_memory_capacity(&mut store, &memory, len)?;
            memory.write(&mut store, 0, json.as_bytes())?;
            let result = validate.call(&mut store, (0, len))?;
            Ok(result != 0)
        } else {
            Ok(false)
        }
    }
}

fn ensure_memory_capacity(store: &mut Store<()>, memory: &Memory, len: i32) -> Result<()> {
    let len_usize = len as usize;
    let current = memory.data_size(&mut *store);
    if len_usize <= current {
        return Ok(());
    }
    let extra_pages = (len_usize - current).div_ceil(0x10000);
    memory
        .grow(store, extra_pages as u64)
        .context("failed to grow booster memory")?;
    Ok(())
}
