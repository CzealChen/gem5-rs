# Platform overlay for the unchanged upstream AM Makefile.
CROSS_COMPILE := riscv64-none-elf-
ARCH_H := arch/riscv.h
COMMON := -march=rv64i -mabi=lp64 -mcmodel=medany -mno-relax -msmall-data-limit=0 -mstrict-align
CFLAGS += $(COMMON) -ffreestanding -fno-pic -fno-pie -fwrapv -ffunction-sections -fdata-sections
ASFLAGS += $(COMMON)
LDSCRIPTS := $(PLATFORM_LD)
LDFLAGS += -melf64lriscv --no-relax --gc-sections --build-id=none
AM_SRCS := platform/gem5rs/start.S platform/gem5rs/runtime.c platform/gem5rs/m5.S
