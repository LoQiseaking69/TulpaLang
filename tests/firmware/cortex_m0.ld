/* BBC micro:bit (nRF51822, Cortex-M0): 256 KB flash, 16 KB RAM. Used to run Tulpa firmware
   under qemu-system-arm -M microbit. QEMU loads every segment, so .data lives in RAM as-is. */
MEMORY {
    FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 256K
    RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 16K
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
