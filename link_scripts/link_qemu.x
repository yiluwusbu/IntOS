/* Memory layout of the LM3S6965 microcontroller */
/* 1K = 1 KiBi = 1024 bytes */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM : ORIGIN = 0x20000000, LENGTH = 64K
}

/* The entry point is the reset handler */
ENTRY(Reset);

EXTERN(EXCEPTIONS);
_stack_start = ORIGIN(RAM) + LENGTH(RAM);
SECTIONS
{
  .vector_table ORIGIN(FLASH) :
  {
    /* First entry: initial Stack Pointer value */
    LONG(ORIGIN(RAM) + LENGTH(RAM));

    /* 15 exception vectors */
    KEEP(*(.vector_table.exceptions)); /* <- NEW */
  } > FLASH
  
  .text :
  {
    *(.text .text.*);
  } > FLASH

  .rodata :
  {
    *(.rodata .rodata.*);
  } > FLASH

  .bss :
  {
    _sbss = .;
    *(.bss .bss.*);
    _ebss = .;
  } > RAM

  .data :  /* AT(ADDR(.rodata) + SIZEOF(.rodata)) */
  {
    _sdata = .;
    *(.data .data.*);
    _edata = .;
  } > RAM AT>FLASH

  _sidata = LOADADDR(.data);

  .pmem : /* AT(ADDR(.data) + SIZEOF(.data)) */
  {
    _spmem = .;
    *(.pmem .pmem.*);
    _epmem = .;
  } > RAM AT>FLASH

  _sipmem = LOADADDR(.pmem);

  .checkpoint_meta :
  {
      . = ALIGN(4);
      *(.checkpoint_meta)
      *(.checkpoint_meta*)
      . = ALIGN(4);
  } > RAM AT>FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx .ARM.exidx.*);
  }
}

PROVIDE(NMI = DefaultExceptionHandler);
PROVIDE(HardFault = DefaultExceptionHandler);
PROVIDE(MemManage = DefaultExceptionHandler);
PROVIDE(BusFault = DefaultExceptionHandler);
PROVIDE(UsageFault = DefaultExceptionHandler);
PROVIDE(SVCall = DefaultExceptionHandler);
PROVIDE(PendSV = DefaultExceptionHandler);
PROVIDE(SysTick = DefaultExceptionHandler);