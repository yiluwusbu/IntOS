ENTRY(Reset)

MEMORY
{
    MCU_MRAM     (rwx) : ORIGIN = 0x00018000, LENGTH = 1998848
    TCM_MAIN_STK (rwx) : ORIGIN = 0x10000000, LENGTH = 4K
    MCU_TCM      (rwx) : ORIGIN = 0x10001000, LENGTH = 389120
    SHARED_SRAM  (rwx) : ORIGIN = 0x10060000, LENGTH = 1048576
}

EXTERN(EXCEPTIONS);

SECTIONS
{
    .vector_table ORIGIN(MCU_MRAM) :
    {
        /* First entry: initial Stack Pointer value */
        LONG(ORIGIN(TCM_MAIN_STK) + LENGTH(TCM_MAIN_STK));

        /* 15 exception vectors */
        KEEP(*(.vector_table.exceptions)); /* <- NEW */
    } > MCU_MRAM

    .text :
    {
        . = ALIGN(4);
        *(.text)
        *(.text*)
        *(.rodata)
        *(.rodata*)
        . = ALIGN(4);
        _etext = .;
    } > MCU_MRAM


    .data :
    {
        . = ALIGN(4);
        _sdata = .;
        *(.data)
        *(.data*)
        . = ALIGN(4);
        _edata = .;
    } > SHARED_SRAM AT>MCU_MRAM

    /* used by startup to initialize data */
    _sidata = LOADADDR(.data);

    .checkpoint_meta :
    {
        . = ALIGN(4);
        *(.checkpoint_meta)
        *(.checkpoint_meta*)
        . = ALIGN(4);
    } > SHARED_SRAM 

    .pmem :
    {
        . = ALIGN(4);
        _spmem = .;
        *(.pmem .pmem.*);
        . = ALIGN(4);
        _epmem = .;
    } > SHARED_SRAM  AT>MCU_MRAM

    _sipmem = LOADADDR(.pmem);


    .bss :
    {
        . = ALIGN(4);
        _sbss = .;
        *(.bss)
        *(.bss*)
        . = ALIGN(4);
        _ebss = .;
    } > SHARED_SRAM

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