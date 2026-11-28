// Versioned RV64I reference process; Spike owns all instruction semantics.
#include <riscv/processor.h>
#include <riscv/simif.h>
#include <array>
#include <charconv>
#include <iostream>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <vector>

#ifndef SPIKE_SOURCE_COMMIT
#error Build with the pinned Spike source identity
#endif

static void require(bool ok, const char* message) {
  if (!ok) throw std::runtime_error(message);
}
static uint64_t number(std::istream& input) {
  std::string text;
  require(bool(input >> text), "missing integer");
  uint64_t value = 0;
  auto [end, error] = std::from_chars(text.data(), text.data() + text.size(), value);
  require(error == std::errc{} && end == text.data() + text.size(), "invalid unsigned integer");
  return value;
}
static void end(std::istream& input) {
  std::string extra;
  require(!(input >> extra), "extra fields");
}

class Memory final : public simif_t {
public:
  reg_t base = 0;
  std::vector<char> ram;
  cfg_t cfg;
  std::map<size_t, processor_t*> harts;
  char* addr_to_mem(reg_t address) override {
    return address >= base && address - base < ram.size() ? ram.data() + address - base : nullptr;
  }
  bool mmio_load(reg_t, size_t, uint8_t*) override { return false; }
  bool mmio_store(reg_t, size_t, const uint8_t*) override { return false; }
  void proc_reset(unsigned) override {}
  const cfg_t& get_cfg() const override { return cfg; }
  const std::map<size_t, processor_t*>& get_harts() const override { return harts; }
  const char* get_symbol(uint64_t) override { return nullptr; }
  size_t offset(uint64_t address, size_t size) {
    require(address >= base && address - base <= ram.size() && size <= ram.size() - (address - base), "memory outside RAM");
    return address - base;
  }
  uint64_t read(uint64_t address, size_t size) {
    require(size <= 8, "memory width exceeds RV64I");
    size_t off = offset(address, size);
    uint64_t value = 0;
    for (size_t i = 0; i < size; ++i) value |= uint64_t(uint8_t(ram[off + i])) << (8 * i);
    return value;
  }
};

static void machine_state(state_t* state) {
  std::cout << ' ' << state->prv << ' ' << state->mstatus->read()
            << ' ' << state->mie->read() << ' ' << state->mip->read()
            << ' ' << state->mtvec->read() << ' ' << state->csrmap.at(0x340)->read()
            << ' ' << state->mepc->read() << ' ' << state->mcause->read()
            << ' ' << state->mtval->read();
}

static void sync(std::istream& input, state_t* state) {
  uint64_t pc = number(input);
  std::array<uint64_t, 32> registers;
  for (auto& reg : registers) reg = number(input);
  end(input);
  require((pc & 3) == 0 && registers[0] == 0, "invalid PC/x0 synchronization state");
  state->pc = pc;
  for (size_t i = 0; i < registers.size(); ++i) state->XPR.write(i, registers[i]);
}

int main() {
  try {
    Memory memory;
    std::unique_ptr<processor_t> cpu;
    std::unique_ptr<FILE, decltype(&fclose)> log(fopen("/dev/null", "w"), fclose);
    require(bool(log), "cannot open commit log sink");
    bool started = false;
    bool machine = false;
    std::array<char, 16385> line;
    while (std::cin.getline(line.data(), line.size())) {
      std::istringstream input(line.data());
      std::string command;
      input >> command;
      if (command == "hello") {
        end(input);
        std::cout << "hello 2 " << SPIKE_SOURCE_COMMIT << " rv64i_zicsr pc_gpr_mem_machine\n";
      } else if (command == "init") {
        require(!cpu, "already initialized");
        std::string isa;
        input >> isa;
        require(isa == "rv64i" || isa == "rv64i_zicsr", "unsupported ISA");
        machine = isa == "rv64i_zicsr";
        memory.base = number(input);
        uint64_t size = number(input);
        require(size && size <= 256 * 1024 * 1024 && !(size % 4096) && !(memory.base % 4096) && memory.base <= UINT64_MAX - size, "invalid RAM layout");
        memory.ram.resize(size, 0);
        memory.cfg.isa = machine ? "RV64I_Zicsr" : "RV64I";
        memory.cfg.priv = "M";
        memory.cfg.mem_layout = {mem_cfg_t(memory.base, size)};
        cpu = std::make_unique<processor_t>(memory.cfg.isa, "M", &memory.cfg, &memory, 0, false, log.get(), std::cerr);
        memory.harts.emplace(0, cpu.get());
        cpu->enable_log_commits();
        if (machine) cpu->get_state()->mstatus->write(3ULL << 11);
        sync(input, cpu->get_state());
        memory.offset(cpu->get_state()->pc, 4);
        std::cout << "ok\n";
      } else if (command == "load") {
        require(cpu && !started, "load requires initialization before execution");
        uint64_t address = number(input);
        std::string bytes;
        input >> bytes;
        end(input);
        require(!bytes.empty() && bytes.size() <= 8192 && bytes.size() % 2 == 0, "invalid load size");
        size_t offset = memory.offset(address, bytes.size() / 2);
        std::vector<char> decoded;
        for (size_t i = 0; i < bytes.size(); i += 2) {
          unsigned byte = 0;
          auto [p, error] = std::from_chars(bytes.data() + i, bytes.data() + i + 2, byte, 16);
          require(error == std::errc{} && p == bytes.data() + i + 2, "invalid hex byte");
          decoded.push_back(char(byte));
        }
        std::copy(decoded.begin(), decoded.end(), memory.ram.begin() + offset);
        std::cout << "ok\n";
      } else if (command == "sync") {
        require(bool(cpu), "not initialized");
        sync(input, cpu->get_state());
        started = true;
        std::cout << "ok\n";
      } else if (command == "pending") {
        require(cpu && machine, "pending requires machine mode");
        uint64_t pending = number(input);
        end(input);
        require(pending <= 1, "invalid timer pending level");
        cpu->get_state()->mip->backdoor_write_with_mask(1ULL << 7, pending << 7);
        std::cout << "ok\n";
      } else if (command == "machine") {
        require(cpu && machine, "machine state requires machine mode");
        end(input);
        auto* state = cpu->get_state();
        std::cout << "machine " << state->pc;
        for (size_t i = 0; i < 32; ++i) std::cout << ' ' << state->XPR[i];
        machine_state(state);
        std::cout << '\n';
      } else if (command == "inspect") {
        require(bool(cpu), "not initialized");
        end(input);
        auto* state = cpu->get_state();
        // Skipping a platform instruction must not hide a DUT that missed an
        // eligible interrupt, especially when that instruction is m5_exit.
        require(!machine || !(state->mstatus->read() & (1ULL << 3)) ||
                !(state->mie->read() & state->mip->read() & (1ULL << 7)),
                "pending timer interrupt before platform instruction");
        auto pc = state->pc;
        std::cout << "instruction " << pc << ' ' << memory.read(pc, 4) << '\n';
      } else if (command == "step") {
        require(bool(cpu), "not initialized");
        end(input);
        started = true;
        auto* state = cpu->get_state();
        uint64_t pc = state->pc;
        uint64_t instruction = pc >= memory.base && pc - memory.base <= memory.ram.size() - 4 ? memory.read(pc, 4) : 0;
        uint64_t retired = state->minstret->read();
        state->log_mem_read.clear(); state->log_mem_write.clear();
        cpu->step(1);
        if (state->minstret->read() != retired + 1) {
          std::cout << "fault " << pc << ' ' << instruction << ' ' << state->mcause->read() << ' ' << state->mepc->read() << ' ' << state->mtval->read() << '\n';
        } else {
          std::cout << "state " << pc << ' ' << instruction << ' ' << state->pc;
          for (size_t i = 0; i < 32; ++i) std::cout << ' ' << state->XPR[i];
          auto& reads = state->log_mem_read;
          auto& writes = state->log_mem_write;
          require(reads.size() + writes.size() <= 1, "memory effects outside RV64I scope");
          std::cout << ' ' << reads.size() + writes.size();
          for (auto [address, ignored, size] : reads) {
            (void)ignored;
            std::cout << " read " << address << ' ' << unsigned(size) << ' ' << memory.read(address, size);
          }
          for (auto [address, value, size] : writes)
            std::cout << " write " << address << ' ' << unsigned(size) << ' ' << value;
          if (machine) {
            std::cout << " machine";
            machine_state(state);
          }
          std::cout << '\n';
        }
      } else if (command == "quit") {
        end(input);
        std::cout << "bye\n" << std::flush;
        return 0;
      } else throw std::runtime_error("unknown command");
      std::cout << std::flush;
    }
    throw std::runtime_error("unexpected EOF or oversized message");
  } catch (const std::exception& error) {
    std::cout << "error " << error.what() << '\n' << std::flush;
    return 1;
  }
}
