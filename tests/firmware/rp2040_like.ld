/* The RP2040's memory budget (2 MB flash, 256 KB RAM) on QEMU's MPS2-AN385 board, for running
   dynamic-tier Tulpa firmware (the VM in tulpa_dyn.c) built for Cortex-M0. */
MEMORY {
    FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 2M
    RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 256K
}
ENTRY(reset_handler)
SECTIONS {
    .text : { KEEP(*(.vectors)) *(.text*) *(.rodata*) . = ALIGN(4); } > FLASH
    .ARM.exidx : { __exidx_start = .; *(.ARM.exidx*) __exidx_end = .; } > FLASH
    .data : { *(.data*) . = ALIGN(4); } > RAM
    .bss (NOLOAD) : { __bss_start__ = .; *(.bss*) *(COMMON) . = ALIGN(4); __bss_end__ = .; } > RAM
    end = .;
    __stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
