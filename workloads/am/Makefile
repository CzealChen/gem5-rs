# Invoke in a prepared build directory via make -f /path/to/this/Makefile.
# Python prepares the source overlay and authoritative platform.ld only.
NAME := $(WORKLOAD)
ARCH := riscv64-gem5rs
SRCS := runner.c $(filter-out src/bench.c,$(shell find src/ -name "*.c" -o -name "*.cc"))
ifeq ($(WORKLOAD),hello)
SRCS := hello.c
endif
ifneq ($(filter $(WORKLOAD),yield-os thread-os),)
SRCS := $(WORKLOAD).c cte.c trap.S
ifeq ($(WORKLOAD),thread-os)
CFLAGS += -DGEM5RS_TIMER
endif
endif
INC_PATH := $(CURDIR)/include
export ARCH AM_HOME PLATFORM_LD
include $(AM_HOME)/Makefile

image: image-dep
	$(OBJDUMP) -d $(IMAGE).elf > $(IMAGE).disasm
	$(READELF) -h -A $(IMAGE).elf > $(IMAGE).attributes
