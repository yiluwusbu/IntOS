/* Memory layout of the MSP430FR5994 */
/* 1K = 1 KiBi = 1024 bytes */
/* LEARAM           : ORIGIN = 0x2C00, LENGTH = 0x0EC8 */
/* LEASTACK         : ORIGIN = 0x3AC8, LENGTH = 0x0138 */
MEMORY
{
  TINYRAM          : ORIGIN = 0xA, LENGTH = 0x0016
  BSL              : ORIGIN = 0x1000, LENGTH = 0x0800
  RAM              : ORIGIN = 0x1C00, LENGTH = 0x2000
  INFOMEM          : ORIGIN = 0x1800, LENGTH = 0x0200
  INFOD            : ORIGIN = 0x1800, LENGTH = 0x0080
  INFOC            : ORIGIN = 0x1880, LENGTH = 0x0080
  INFOB            : ORIGIN = 0x1900, LENGTH = 0x0080
  INFOA            : ORIGIN = 0x1980, LENGTH = 0x0080
  FRAM (rwx)       : ORIGIN = 0x4000, LENGTH = 0xBF80 /* END=0xFF7F, size 49024 */
  HIFRAM (rxw)     : ORIGIN = 0x00010000, LENGTH = 0x00033FFF
  JTAGSIGNATURE    : ORIGIN = 0xFF80, LENGTH = 0x0004
  BSLSIGNATURE     : ORIGIN = 0xFF84, LENGTH = 0x0004
  IPESIGNATURE     : ORIGIN = 0xFF88, LENGTH = 0x0008
  VECTORS          : ORIGIN = 0xFF90, LENGTH = 0x70
}

/* The entry point is the reset handler */
ENTRY(Reset);
EXTERN(VECTOR_TABLE);
EXTERN(DefaultHandler);

/* _stack_start = ORIGIN(FRAM) + LENGTH(FRAM); */
_stack_start = ORIGIN(RAM) + LENGTH(RAM);
SECTIONS
{
  .jtagsignature      : {} > JTAGSIGNATURE
  .bslsignature       : {} > BSLSIGNATURE
  .ipe :
  {
    KEEP (*(.ipesignature))
    KEEP (*(.jtagpassword))
  } > IPESIGNATURE

  .vector_table ORIGIN(VECTORS) : ALIGN(2)
  {
    /* 56 exception vectors */
    KEEP(*(.vector_table.exceptions)); 
  } > VECTORS
  
  .text ORIGIN(FRAM) : 
  {
    *(.text .text.*);
  } > FRAM

  .rodata : ALIGN(2)
  {
    *(.rodata .rodata.*);
    . = ALIGN(2);
  } > FRAM

  .bss : ALIGN(2)
  {
    _sbss = .;
    *(.bss .bss.*);
    . = ALIGN(2);
    _ebss = .;
  } > FRAM

  .data : ALIGN(2)
  {
    _sdata = .;
    *(.data .data.*);
    . = ALIGN(2);
    _edata = .;
  } > FRAM

  _sidata = LOADADDR(.data);

  .pmem : ALIGN(2)
  {
    _spmem = .;
    *(.pmem .pmem.*);
    . = ALIGN(2);
    _epmem = .;
  } > FRAM 

  _sipmem = LOADADDR(.pmem);

}