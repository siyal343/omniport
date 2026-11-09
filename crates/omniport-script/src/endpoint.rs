use omniport_core::{
    AsyncEndpoint, EndpointId, FrameFlags, PinState, SerialConfig, SerialEvent,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex};

use crate::engine::ScriptEngine;

pub struct ScriptEndpoint {
    id: EndpointId,
    name: String,
    config: SerialConfig,
    engine: Arc<Mutex<ScriptEngine>>,
    outbound_tx: mpsc::Sender<SerialEvent>,
    outbound_rx: mpsc::Receiver<SerialEvent>,
    timer_cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ScriptEndpoint {
    pub fn new(
        id: EndpointId,
        name: impl Into<String>,
        config: SerialConfig,
        engine: ScriptEngine,
    ) -> Self {
        let (outbound_tx, outbound_rx) = mpsc::channel(1024);
        Self {
            id,
            name: name.into(),
            config,
            engine: Arc::new(Mutex::new(engine)),
            outbound_tx,
            outbound_rx,
            timer_cancel_tx: None,
        }
    }

    /// Spawns a periodic background timer that triggers Lua `on_timer(timer_id)`
    pub fn start_timer(&mut self, timer_id: u32, interval: Duration) {
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
        self.timer_cancel_tx = Some(cancel_tx);

        let engine = self.engine.clone();
        let tx = self.outbound_tx.clone();
        let source_id = self.id;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = &mut cancel_rx => break,
                    _ = ticker.tick() => {
                        let res = {
                            let mut eng = engine.lock().await;
                            if let Ok(responses) = eng.dispatch_timer(timer_id) {
                                let pins = eng.current_pins();
                                Some((responses, pins))
                            } else {
                                None
                            }
                        };

                        if let Some((responses, pins)) = res {
                            let now_ns = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos() as u64;

                            for resp in responses {
                                let evt = SerialEvent {
                                    timestamp_ns: now_ns,
                                    source_id,
                                    flags: FrameFlags::DIR_TX,
                                    pins,
                                    payload: resp,
                                };
                                if tx.send(evt).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn engine(&self) -> Arc<Mutex<ScriptEngine>> {
        self.engine.clone()
    }
}

impl Drop for ScriptEndpoint {
    fn drop(&mut self) {
        if let Some(cancel) = self.timer_cancel_tx.take() {
            let _ = cancel.send(());
        }
    }
}

impl AsyncEndpoint for ScriptEndpoint {
    fn id(&self) -> EndpointId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = std::io::Result<SerialEvent>> + Send + '_>> {
        Box::pin(async move {
            match self.outbound_rx.recv().await {
                Some(evt) => Ok(evt),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Script endpoint closed",
                )),
            }
        })
    }

    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let (responses, current_pins) = {
                let mut eng = self.engine.lock().await;
                let responses = eng.dispatch_rx(&event.payload, event.pins).map_err(|e| {
                    std::io::Error::other(format!("Lua script error: {e}"))
                })?;
                let pins = eng.current_pins();
                (responses, pins)
            };

            let now_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;

            for resp in responses {
                let out_evt = SerialEvent {
                    timestamp_ns: now_ns,
                    source_id: self.id,
                    flags: FrameFlags::DIR_TX,
                    pins: current_pins,
                    payload: resp,
                };
                self.outbound_tx
                    .send(out_evt)
                    .await
                    .map_err(|_| std::io::Error::other("Failed to enqueue script output"))?;
            }

            Ok(())
        })
    }

    fn set_pins(&mut self, pins: PinState) -> std::io::Result<()> {
        let engine = self.engine.clone();
        let tx = self.outbound_tx.clone();
        let source_id = self.id;

        tokio::spawn(async move {
            let res = {
                let mut eng = engine.lock().await;
                if let Ok(responses) = eng.dispatch_pin_change(pins) {
                    let current_pins = eng.current_pins();
                    Some((responses, current_pins))
                } else {
                    None
                }
            };

            if let Some((responses, current_pins)) = res {
                let now_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;

                for resp in responses {
                    let out_evt = SerialEvent {
                        timestamp_ns: now_ns,
                        source_id,
                        flags: FrameFlags::DIR_TX | FrameFlags::PIN_CHANGE,
                        pins: current_pins,
                        payload: resp,
                    };
                    let _ = tx.send(out_evt).await;
                }
            }
        });

        Ok(())
    }

    fn configure(&mut self, cfg: SerialConfig) -> std::io::Result<()> {
        self.config = cfg;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ScriptConfig;
    use bytes::Bytes;

    #[tokio::test]
    async fn test_script_endpoint_roundtrip_io() {
        let mut engine = ScriptEngine::new(ScriptConfig::default()).unwrap();
        let script = r#"
            function on_rx(payload, pins)
                serial.send("PONG:" .. payload)
                serial.set_pins({ cts = true })
            end
        "#;
        engine.load_script(script).unwrap();

        let mut ep = ScriptEndpoint::new(42, "LuaSim", SerialConfig::default(), engine);

        let input_evt = SerialEvent {
            timestamp_ns: 1000,
            source_id: 1,
            flags: FrameFlags::empty(),
            pins: PinState::RTS,
            payload: Bytes::copy_from_slice(b"TEST"),
        };

        ep.send(input_evt).await.unwrap();

        let output_evt = ep.recv().await.unwrap();
        assert_eq!(&output_evt.payload[..], b"PONG:TEST");
        assert_eq!(output_evt.source_id, 42);
        assert!(output_evt.pins.contains(PinState::CTS));
    }

    #[tokio::test]
    async fn test_script_endpoint_periodic_timer() {
        let mut engine = ScriptEngine::new(ScriptConfig::default()).unwrap();
        let script = r#"
            local n = 0
            function on_timer(id)
                n = n + 1
                serial.send("TICK_" .. n)
            end
        "#;
        engine.load_script(script).unwrap();

        let mut ep = ScriptEndpoint::new(43, "TimerSim", SerialConfig::default(), engine);
        ep.start_timer(1, Duration::from_millis(10));

        let t1 = ep.recv().await.unwrap();
        assert_eq!(&t1.payload[..], b"TICK_1");

        let t2 = ep.recv().await.unwrap();
        assert_eq!(&t2.payload[..], b"TICK_2");
    }
}
