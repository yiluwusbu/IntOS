target remote :2331
load
monitor semihosting enable
monitor semihosting IOClient 3
monitor reset 0
set disassemble-next-line on