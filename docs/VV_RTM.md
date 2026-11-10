# Verification & Validation Plan & Requirements Traceability Matrix

| Req ID | Module | Verification Method | Target Criteria |
| :--- | :--- | :--- | :--- |
| FR-101..104 | `omniport-hal-*` | Integration + HIL | Port lifecycle < 200 ms, Baud within 1.5% |
| FR-201..203 | `omniport-core` | Property Fuzzing | 100% frame recovery, zero merged byte interleaving |
| FR-301..302 | `omniport-core` | Statistical Test | Jitter and BER match configured parameters |
| FR-401 | `omniport-capture` | Benchmark + Unit | $O(1)$ viewport seek < 100 us at 50M packets |
| FR-402 | `omniport-dissect` | Unit + Fuzz Test | Decoded fields match packed C reference |
