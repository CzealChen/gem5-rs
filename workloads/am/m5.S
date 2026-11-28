.section .text
.balign 4
.option norvc
.global m5_rpns
.type m5_rpns, @function
m5_rpns:
.long 0x0e00007b
ret
.size m5_rpns, .-m5_rpns
.global m5_exit
.type m5_exit, @function
m5_exit:
.long 0x4200007b
ret
.size m5_exit, .-m5_exit
.global m5_fail
.type m5_fail, @function
m5_fail:
.long 0x4400007b
ret
.size m5_fail, .-m5_fail
.global m5_work_begin
.type m5_work_begin, @function
m5_work_begin:
.long 0xb400007b
ret
.size m5_work_begin, .-m5_work_begin
.global m5_work_end
.type m5_work_end, @function
m5_work_end:
.long 0xb600007b
ret
.size m5_work_end, .-m5_work_end
