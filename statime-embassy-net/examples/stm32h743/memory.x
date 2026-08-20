MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 2048K
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 128K
  SRAM3 (rwx) : ORIGIN = 0x30040000, LENGTH = 32K
}

_stack_start = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS {
  .sram3 (NOLOAD) : ALIGN(4) {
    *(.sram3 .sram3.*);
    . = ALIGN(4);
  } > SRAM3
}
