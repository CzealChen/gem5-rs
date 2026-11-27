use super::{run, validate};
use serde_json::{Value, json};

fn edge(src: u64, out: u64, dst: u64, input: u64) -> Value {
    json!({"source": {"graph_id": 7, "node": src, "port": out},
           "target": {"graph_id": 7, "node": dst, "port": input}})
}

fn graph(nodes: Vec<Value>, connections: Vec<Value>) -> Value {
    json!({"graph_id": 7, "nodes": nodes, "connections": connections})
}

fn run_board(graph: &Value, events: u64) -> Value {
    serde_json::from_str(&run(&graph.to_string(), events).unwrap()).unwrap()
}

fn traffic(name: &str, operations: Vec<Value>) -> Value {
    json!({"kind": "traffic", "name": name, "operations": operations})
}

fn fix_ram(name: &str) -> Value {
    json!({"kind": "fixed_ram", "name": name, "size_bytes": 64, "latency": 10, "capacity": 1})
}

fn read(at: u64, addr: u64) -> Value {
    json!({"kind": "read", "at": at, "addr": addr, "size_bytes": 1})
}

fn write(at: u64, addr: u64, data: u8) -> Value {
    json!({"kind": "write", "at": at, "addr": addr, "data": [data]})
}

fn two_sources() -> Value {
    graph(
        vec![
            traffic("reader", vec![read(0, 0)]),
            traffic("writer", vec![write(0, 0, 65)]),
            json!({"kind": "router", "name": "bus", "inputs": 2,
                   "ranges": [{"start": 0, "size_bytes": 64}], "priority": [1, 0]}),
            fix_ram("memory"),
        ],
        vec![edge(0, 0, 2, 0), edge(1, 0, 2, 1), edge(2, 0, 3, 0)],
    )
}

#[test]
fn ingress_priority_controls_read_visibility_and_backpressure() {
    let board = two_sources();
    assert_eq!(validate(&board.to_string()), Ok(()));
    let result = run_board(&board, 100);
    assert_eq!(result["stop_reason"], "idle");
    assert_eq!(result["final_tick"], 20);
    assert_eq!(
        result["transactions"],
        json!([
            {"id": 0, "source": "writer", "addr": 0, "kind": "write", "size_bytes": 1,
             "accepted_at": 0, "completed_at": 10, "data": null, "fault": null},
            {"id": 0, "source": "reader", "addr": 0, "kind": "read", "size_bytes": 1,
             "accepted_at": 10, "completed_at": 20, "data": [65], "fault": null},
        ])
    );
    assert_eq!(
        result["stats"],
        json!({"events": 10, "accepted": 2, "rejected": 1, "retries": 1,
                                    "fetches": 0, "reads": 1, "writes": 1, "committed": 0})
    );
    assert_eq!(run_board(&board, 100), result);

    let mut reversed = board;
    reversed["nodes"][2]["priority"] = json!([0, 1]);
    assert_eq!(
        run_board(&reversed, 100)["transactions"][0]["data"],
        json!([0])
    );
}

#[test]
fn separate_banks_complete_concurrently_while_the_busy_bank_retries() {
    let board = graph(
        vec![
            traffic(
                "probe",
                vec![write(3, 0, 42), read(3, 0), write(3, 8, 99), read(4, 8)],
            ),
            json!({"kind": "dual_bank_ram", "name": "memory", "size_bytes": 32, "latency": 5, "bank_bytes": 8}),
        ],
        vec![edge(0, 0, 1, 0)],
    );
    let result = run_board(&board, 100);
    let trace = result["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tx| json!([tx["id"], tx["accepted_at"], tx["completed_at"], tx["data"]]))
        .collect::<Vec<_>>();
    assert_eq!(
        trace,
        vec![
            json!([0, 3, 8, null]),
            json!([2, 3, 8, null]),
            json!([1, 8, 13, [42]]),
            json!([3, 8, 13, [99]])
        ]
    );
    assert_eq!(result["stats"]["rejected"], 2);
    assert_eq!(result["stats"]["retries"], 2);
}

#[test]
fn graph_errors_identify_the_port() {
    let valid = two_sources();
    let cases = [
        (
            vec![edge(0, 0, 2, 0), edge(1, 0, 2, 0), edge(2, 0, 3, 0)],
            "bus.inputs[0] is already connected",
        ),
        (
            vec![edge(0, 0, 2, 0), edge(0, 0, 2, 1), edge(2, 0, 3, 0)],
            "reader.requests is already connected",
        ),
        (
            vec![edge(0, 0, 2, 0), edge(1, 0, 2, 1)],
            "bus.outputs[0] is unconnected",
        ),
        (vec![edge(3, 0, 2, 0)], "memory: invalid output port 0"),
        (vec![edge(0, 0, 1, 0)], "writer: invalid input port 0"),
        (vec![edge(0, 0, 2, 2)], "bus: invalid input port 2"),
        (
            vec![edge(0, 0, 101, 2)],
            "node 101 port 2: unknown instance",
        ),
    ];
    for (connections, expected) in cases {
        let mut board = valid.clone();
        board["connections"] = json!(connections);
        let graph = board.to_string();
        assert!(validate(&graph).unwrap_err().contains(expected));
        assert!(run(&graph, 100).unwrap_err().contains(expected));
    }
    let mut cross = valid.clone();
    cross["connections"][0]["source"]["graph_id"] = json!(8);
    assert!(
        validate(&cross.to_string())
            .unwrap_err()
            .contains("reader.requests belongs to graph 8")
    );

    let mut priority = valid;
    priority["nodes"][2]["priority"] = json!([0, 0]);
    assert!(
        validate(&priority.to_string())
            .unwrap_err()
            .contains("bus.inputs[0]: duplicate ingress")
    );
}

#[test]
fn router_delivers_requests_to_each_declared_window() {
    let board = graph(
        vec![
            traffic(
                "probe",
                vec![
                    write(0, 192, 88),
                    write(0, 128, 41),
                    read(5, 256),
                    read(11, 128),
                    read(11, 192),
                ],
            ),
            json!({"kind": "router", "name": "bus", "inputs": 1,
                       "ranges": [{"start": 192, "size_bytes": 64}, {"start": 128, "size_bytes": 64}], "priority": [0]}),
            fix_ram("right"),
            fix_ram("left"),
        ],
        vec![edge(0, 0, 1, 0), edge(1, 0, 2, 0), edge(1, 1, 3, 0)],
    );
    let result = run_board(&board, 100);
    assert_eq!(result["transactions"][0]["fault"], "Unmapped");
    assert_eq!(result["transactions"][0]["completed_at"], 6);
    assert_eq!(result["transactions"][3]["data"], json!([41]));
    assert_eq!(result["transactions"][4]["data"], json!([88]));

    let mut overlap = board.clone();
    overlap["nodes"][1]["ranges"][1]["start"] = json!(176);
    assert!(
        validate(&overlap.to_string())
            .unwrap_err()
            .contains("bus.outputs[1] overlaps bus.outputs[0]")
    );
    let mut mismatch = board;
    mismatch["nodes"][2]["size_bytes"] = json!(128);
    assert!(
        validate(&mismatch.to_string())
            .unwrap_err()
            .contains("bus.outputs[0] range must match right.input")
    );
}

#[test]
fn disjoint_paths_require_a_shared_router() {
    let board = graph(
        vec![
            traffic("left", vec![]),
            traffic("right", vec![]),
            fix_ram("ram0"),
            fix_ram("ram1"),
        ],
        vec![edge(0, 0, 2, 0), edge(1, 0, 3, 0)],
    );
    assert!(
        validate(&board.to_string())
            .unwrap_err()
            .contains("left.requests and right.requests")
    );
}

#[test]
fn event_budget_counts_acceptance_before_response_delivery() {
    let board = graph(
        vec![traffic("probe", vec![read(0, 0)]), fix_ram("memory")],
        vec![edge(0, 0, 1, 0)],
    );
    let before = run_board(&board, 4);
    assert_eq!(before["stop_reason"], "event_limit");
    assert_eq!(before["final_tick"], 10);
    assert_eq!(before["stats"]["reads"], 1);
    assert_eq!(before["transactions"], json!([]));
    let delivered = run_board(&board, 5);
    assert_eq!(delivered["stop_reason"], "idle");
    assert_eq!(delivered["transactions"][0]["data"], json!([0]));
}

fn rv64_elf(instructions: &[u32]) -> Vec<u8> {
    let bytes = instructions.len() as u64 * 4;
    let mut elf = vec![0; 120];
    elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    elf[16..18].copy_from_slice(&2u16.to_le_bytes());
    elf[18..20].copy_from_slice(&243u16.to_le_bytes());
    elf[20..24].copy_from_slice(&1u32.to_le_bytes());
    elf[24..32].copy_from_slice(&0x100u64.to_le_bytes());
    elf[32..40].copy_from_slice(&64u64.to_le_bytes());
    elf[52..54].copy_from_slice(&64u16.to_le_bytes());
    elf[54..56].copy_from_slice(&56u16.to_le_bytes());
    elf[56..58].copy_from_slice(&1u16.to_le_bytes());
    elf[64..68].copy_from_slice(&1u32.to_le_bytes());
    elf[68..72].copy_from_slice(&5u32.to_le_bytes());
    elf[72..80].copy_from_slice(&120u64.to_le_bytes());
    elf[80..88].copy_from_slice(&0x100u64.to_le_bytes());
    elf[96..104].copy_from_slice(&bytes.to_le_bytes());
    elf[104..112].copy_from_slice(&(bytes + 4).to_le_bytes());
    for instruction in instructions {
        elf.extend_from_slice(&instruction.to_le_bytes());
    }
    elf
}

fn cpu_board(instructions: &[u32]) -> Value {
    graph(
        vec![
            json!({"kind": "rv64_core", "name": "core", "elf": rv64_elf(instructions), "stack_pointer": 0x140, "cycle": 2}),
            json!({"kind": "router", "name": "bus", "inputs": 1,
                       "ranges": [{"start": 0x100, "size_bytes": 64}, {"start": 0x200, "size_bytes": 8}], "priority": [0]}),
            fix_ram("boot"),
            json!({"kind": "exit", "name": "shutdown", "addr": 0x200, "width_bytes": 8}),
        ],
        vec![edge(0, 0, 1, 0), edge(1, 0, 2, 0), edge(1, 1, 3, 0)],
    )
}

#[test]
fn elf_boot_commits_the_exit_store() {
    // addi x3,x0,512; addi x4,x2,0; sd x4,0(x3).
    let board = cpu_board(&[0x20000193, 0x00010213, 0x0041b023]);
    let result = run_board(&board, 100);
    assert_eq!(result["stop_reason"], "exit");
    assert_eq!(result["exit_code"], 0x140);
    let commits = result["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 3);
    assert_eq!(
        commits[1]["writeback"],
        json!({"register": 4, "value": 0x140})
    );
    assert_eq!(commits[2]["pc"], 0x108);
    assert_eq!(commits[2]["writeback"], Value::Null);
    assert_eq!(commits[2]["at"], result["transactions"][3]["completed_at"]);
    assert_eq!(result["stats"]["fetches"], 3);
    assert_eq!(result["stats"]["writes"], 1);
}

#[test]
fn core_reports_faulting_pc_and_address() {
    // ld x1,0(x0), with no mapping at zero.
    let result = run_board(&cpu_board(&[0x00003083]), 100);
    assert_eq!(result["stop_reason"], "fault");
    assert_eq!(result["fault"]["pc"], 0x100);
    assert_eq!(result["fault"]["addr"], 0);
    assert_eq!(result["transactions"][1]["fault"], "Unmapped");
}
