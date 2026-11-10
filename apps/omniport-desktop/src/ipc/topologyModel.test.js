import test from "node:test";
import assert from "node:assert/strict";
import {
  DemoTopologyStore,
  TopologyMutationError,
  addRouteMutation,
  cloneTopologySnapshot,
  createDemoTopologySnapshot,
  createVirtualPairMutation,
  removeEndpointMutation,
  removeRouteMutation,
} from "./topologyModel.ts";
import { nextVirtualPairPortNames } from "../components/Topology/topologyData.ts";

function deepFreeze(value) {
  if (value && typeof value === "object") {
    for (const key of Object.keys(value)) deepFreeze(value[key]);
    Object.freeze(value);
  }
  return value;
}

test("demo seed mirrors the three canonical demo endpoints and two routes", () => {
  const snapshot = createDemoTopologySnapshot();
  assert.deepEqual(
    snapshot.endpoints.map((endpoint) => endpoint.name),
    ["HOST UART", "DUT VIRTUAL PAIR", "WIRESHARK EXTCAP"]
  );
  assert.deepEqual(
    snapshot.endpoints.map((endpoint) => endpoint.id),
    [1, 2, 3]
  );
  assert.equal(snapshot.routes.length, 2);
  assert.deepEqual(
    snapshot.routes.map((route) => route.route_id),
    [100, 101]
  );
  assert.equal(snapshot.routes[0].source_id, 1);
  assert.deepEqual(snapshot.routes[0].target_ids, [2]);
  assert.equal(snapshot.routes[0].bidirectional, true);
  assert.equal(snapshot.routes[0].fault?.bit_error_rate, 1e-4);
  assert.deepEqual(snapshot.routes[1].target_ids, [3]);
  assert.equal(snapshot.routes[1].fault, null);
});

test("createVirtualPair allocates endpoint ids + a bidirectional pair route", () => {
  const before = deepFreeze(createDemoTopologySnapshot());
  const after = createVirtualPairMutation(before, {
    portA: "COM10",
    portB: "COM11",
  });

  assert.equal(after.endpoints.length, 5);
  assert.deepEqual(
    after.endpoints.slice(3).map((endpoint) => ({ id: endpoint.id, name: endpoint.name })),
    [
      { id: 4, name: "COM10" },
      { id: 5, name: "COM11" },
    ]
  );
  assert.equal(after.endpoints[3].port_type, "VirtualPair");
  assert.equal(after.endpoints[3].baud_rate, 115200);
  assert.equal(after.routes.length, 3);
  const pairRoute = after.routes[2];
  assert.equal(pairRoute.route_id, 102);
  assert.equal(pairRoute.source_id, 4);
  assert.deepEqual(pairRoute.target_ids, [5]);
  assert.equal(pairRoute.bidirectional, true);
  assert.equal(pairRoute.fault, null);
  // input untouched
  assert.equal(before.endpoints.length, 3);
  assert.equal(before.routes.length, 2);
});

test("nextVirtualPairPortNames yields COM10/COM11 then COM12/COM13", () => {
  const seed = createDemoTopologySnapshot();
  assert.deepEqual(nextVirtualPairPortNames(seed), { portA: "COM10", portB: "COM11" });
  const withPair = createVirtualPairMutation(seed, { portA: "COM10", portB: "COM11" });
  assert.deepEqual(nextVirtualPairPortNames(withPair), { portA: "COM12", portB: "COM13" });
});

test("createVirtualPair validates empty and duplicate port names", () => {
  const seed = createDemoTopologySnapshot();
  assert.throws(
    () => createVirtualPairMutation(seed, { portA: " ", portB: "COM11" }),
    TopologyMutationError
  );
  assert.throws(
    () => createVirtualPairMutation(seed, { portA: "COM10", portB: "COM10" }),
    TopologyMutationError
  );
});

test("addRoute validates unknown endpoints, self-loops, duplicates and empty targets", () => {
  const seed = createDemoTopologySnapshot();
  assert.throws(
    () => addRouteMutation(seed, { sourceId: 99, targetIds: [2] }),
    /Unknown source endpoint 99/
  );
  assert.throws(
    () => addRouteMutation(seed, { sourceId: 2, targetIds: [99] }),
    /Unknown target endpoint 99/
  );
  assert.throws(
    () => addRouteMutation(seed, { sourceId: 2, targetIds: [2] }),
    /Self-loop/
  );
  assert.throws(() => addRouteMutation(seed, { sourceId: 2, targetIds: [] }), /at least one target/);
  assert.throws(
    () => addRouteMutation(seed, { sourceId: 2, targetIds: [3, 3] }),
    /Duplicate target/
  );
  // 1 <-> 2 already exists (bidirectional), both ordered pairs are duplicates.
  assert.throws(() => addRouteMutation(seed, { sourceId: 2, targetIds: [1] }), /already exists/);
  assert.throws(() => addRouteMutation(seed, { sourceId: 1, targetIds: [2] }), /already exists/);
});

test("addRoute appends a new route with cloned fault config", () => {
  const seed = deepFreeze(createDemoTopologySnapshot());
  const after = addRouteMutation(seed, {
    sourceId: 2,
    targetIds: [3],
    bidirectional: true,
    fault: {
      fixed_delay: { secs: 0, nanos: 1_000_000 },
      jitter_mean: { secs: 0, nanos: 0 },
      jitter_std_dev: { secs: 0, nanos: 0 },
      rate_limit_bps: null,
      disconnected: false,
      bit_error_rate: 0.5,
      gilbert_elliott: null,
      corrupt_nth_crc: 3,
    },
  });
  assert.equal(after.routes.length, 3);
  const route = after.routes[2];
  assert.equal(route.route_id, 102);
  assert.equal(route.source_id, 2);
  assert.deepEqual(route.target_ids, [3]);
  assert.equal(route.bidirectional, true);
  assert.equal(route.fault?.bit_error_rate, 0.5);
  assert.equal(route.fault?.corrupt_nth_crc, 3);
});

test("removeRoute removes exactly one route and rejects unknown ids", () => {
  const seed = createDemoTopologySnapshot();
  const after = removeRouteMutation(seed, 100);
  assert.deepEqual(
    after.routes.map((route) => route.route_id),
    [101]
  );
  assert.throws(() => removeRouteMutation(seed, 999), /Unknown route 999/);
});

test("removeEndpoint prunes routes and target ids like the Rust Topology", () => {
  const seed = createDemoTopologySnapshot();

  // Endpoint 2 is the target of bidirectional route 100: target list empties -> route pruned.
  const withoutTwo = removeEndpointMutation(seed, 2);
  assert.deepEqual(
    withoutTwo.endpoints.map((endpoint) => endpoint.id),
    [1, 3]
  );
  assert.deepEqual(
    withoutTwo.routes.map((route) => route.route_id),
    [101]
  );

  // Endpoint 1 is a source: both 100 and 101 disappear.
  const withoutOne = removeEndpointMutation(seed, 1);
  assert.deepEqual(withoutOne.endpoints.map((endpoint) => endpoint.id), [2, 3]);
  assert.equal(withoutOne.routes.length, 0);

  // Multi-target route keeps its other targets.
  const multi = {
    endpoints: seed.endpoints,
    routes: [{ route_id: 7, source_id: 1, target_ids: [2, 3], bidirectional: false, fault: null }],
  };
  const prunedTargets = removeEndpointMutation(multi, 3);
  assert.deepEqual(prunedTargets.routes[0].target_ids, [2]);
  assert.throws(() => removeEndpointMutation(seed, 99), /Unknown endpoint 99/);
});

test("mutations never share state with their input snapshot", () => {
  const seed = createDemoTopologySnapshot();
  const frozen = deepFreeze(cloneTopologySnapshot(seed));

  const pair = createVirtualPairMutation(frozen, { portA: "COM10", portB: "COM11" });
  assert.notEqual(pair.endpoints, frozen.endpoints);
  assert.notEqual(pair.endpoints[0], frozen.endpoints[0]);
  assert.notEqual(pair.routes, frozen.routes);
  assert.notEqual(pair.routes[0].target_ids, frozen.routes[0].target_ids);
  assert.notEqual(pair.routes[0].fault, frozen.routes[0].fault);

  // Mutating the returned snapshot must not affect the frozen input.
  pair.endpoints[0].name = "MUTATED";
  pair.routes[0].target_ids.push(42);
  assert.equal(frozen.endpoints[0].name, "HOST UART");
  assert.deepEqual(frozen.routes[0].target_ids, [2]);
});

test("DemoTopologyStore keeps a canonical in-memory snapshot across mutations", () => {
  const store = new DemoTopologyStore();
  const initial = store.getSnapshot();
  assert.equal(initial.endpoints.length, 3);

  const afterPair = store.createVirtualPair({ portA: "COM10", portB: "COM11" });
  assert.equal(afterPair.endpoints.length, 5);
  assert.equal(afterPair.routes.length, 3);

  const afterRoute = store.addRoute({ sourceId: 2, targetIds: [4] });
  assert.equal(afterRoute.routes.length, 4);

  const afterRemove = store.removeRoute(101);
  assert.deepEqual(
    afterRemove.routes.map((route) => route.route_id),
    afterRoute.routes.filter((route) => route.route_id !== 101).map((route) => route.route_id)
  );

  const afterEndpoint = store.removeEndpoint(5);
  assert.equal(afterEndpoint.endpoints.some((endpoint) => endpoint.id === 5), false);
  assert.equal(
    afterEndpoint.routes.some((route) => route.target_ids.includes(5)),
    false
  );

  // getSnapshot is a copy: callers cannot corrupt store state.
  const copy = store.getSnapshot();
  copy.endpoints[0].name = "HACKED";
  assert.equal(store.getSnapshot().endpoints[0].name, "HOST UART");

  assert.throws(() => store.removeRoute(999), TopologyMutationError);
  assert.throws(() => store.removeEndpoint(999), /Unknown endpoint 999/);
});
