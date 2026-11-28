// A0 capability probe, not the production reference protocol.
#include <riscv/processor.h>
#include <riscv/simif.h>
#include <array>
#include <cstring>
#include <iostream>
#include <stdexcept>

class Memory final : public simif_t {
public:
  static constexpr reg_t base = 0x80000000;
  std::array<char, 4096> ram{};
  cfg_t cfg;
  std::map<size_t, processor_t*> harts;
  char* addr_to_mem(reg_t address) override {
    return address >= base && address - base < ram.size()
      ? ram.data() + (address - base) : nullptr;
  }
  bool mmio_load(reg_t, size_t, uint8_t*) override { return false; }
  bool mmio_store(reg_t, size_t, const uint8_t*) override { return false; }
  void proc_reset(unsigned) override {}
  const cfg_t& get_cfg() const override { return cfg; }
  const std::map<size_t, processor_t*>& get_harts() const override { return harts; }
  const char* get_symbol(uint64_t) override { return nullptr; }
};

static void require(bool ok, const char* message) {
  if (!ok) throw std::runtime_error(message);
}

static void snapshot(state_t* state) {
  std::cout << state->pc;
  for (size_t i = 0; i < 32; ++i) std::cout << ' ' << state->XPR[i];
  std::cout << '\n';
}

int main() {
  try {
    Memory memory;
    memory.cfg.isa = "RV64I";
    memory.cfg.priv = "M";
    memory.cfg.mem_layout = {mem_cfg_t(Memory::base, memory.ram.size())};
    // addi a0,zero,7; addi a1,a0,5; sd a1,256(x1); ld a2,256(x1);
    // add a3,a0,a2; ebreak. Bytes are explicitly little endian.
    constexpr uint32_t program[] = {
      0x00700513, 0x00550593, 0x10b0b023, 0x1000b603, 0x00c506b3, 0x00100073
    };
    for (size_t i = 0; i < std::size(program); ++i)
      for (size_t j = 0; j < 4; ++j)
        memory.ram[i * 4 + j] = (program[i] >> (8 * j)) & 255;
    processor_t cpu("RV64I", "M", &memory.cfg, &memory, 0, false, stderr, std::cerr);
    memory.harts.emplace(0, &cpu);
    cpu.enable_log_commits();
    auto* state = cpu.get_state();
    state->pc = Memory::base;
    for (size_t i = 0; i < 32; ++i) state->XPR.write(i, 0);
    state->XPR.write(1, Memory::base);
    for (size_t i = 0; i < 4; ++i) {
      cpu.step(1);
      require(state->pc == Memory::base + (i + 1) * 4, "single step PC");
      if (i == 2) {
        require(state->log_mem_write.size() == 1, "store observation unavailable");
        auto [address, value, size] = state->log_mem_write.front();
        require(address == Memory::base + 256 && value == 12 && size == 8, "store effect");
      }
      if (i == 3) {
        require(state->log_mem_read.size() == 1, "load observation unavailable");
        auto [address, value, size] = state->log_mem_read.front();
        (void)value; // Spike load log does not promise the returned data.
        require(address == Memory::base + 256 && size == 8 && state->XPR[12] == 12, "load effect");
      }
      snapshot(state);
    }
    // Simulate a completed platform operation by replacing every GPR and PC.
    for (size_t i = 0; i < 32; ++i) state->XPR.write(i, i * 3);
    state->pc = Memory::base + 16;
    require(state->XPR[0] == 0, "x0 invariant");
    cpu.step(1);
    require(state->XPR[13] == 66, "execution after full state synchronization");
    snapshot(state);
    cpu.step(1);
    require(state->mcause->read() == 3 && state->mepc->read() == Memory::base + 20,
            "breakpoint cause/PC unavailable");
    std::cout << "breakpoint 3 " << state->mepc->read() << '\n';
  } catch (const std::exception& error) {
    std::cerr << "Spike capability probe: " << error.what() << '\n';
    return 1;
  }
}
