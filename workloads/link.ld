OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY {
    RAM (rwx) : ORIGIN = 0x80000000, LENGTH = 4K
}

PHDRS {
    code PT_LOAD FLAGS(5);
    data PT_LOAD FLAGS(6);
}

SECTIONS {
    .text : {
        KEEP(*(.text.start))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > RAM :code

    .data : ALIGN(8) {
        *(.data .data.* .sdata .sdata.*)
    } > RAM :data

    .bss (NOLOAD) : ALIGN(8) {
        *(.bss .bss.* .sbss .sbss.* COMMON)
    } > RAM :data

    __stack_top = ORIGIN(RAM) + LENGTH(RAM) - 16;
    ASSERT(. <= __stack_top - 256, "image overlaps reserved stack")
}
