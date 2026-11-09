MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 2048K
  /* Includes Xarxa's packet pool: Ethernet DMA cannot access DTCM. */
  RAM   (rwx) : ORIGIN = 0x24000000, LENGTH = 512K
  SRAM3 (rwx) : ORIGIN = 0x30040000, LENGTH = 32K
}

_stack_start = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS {
  .sram3 (NOLOAD) : ALIGN(4) {
    *(.sram3 .sram3.*);
    . = ALIGN(4);
  } > SRAM3
}
