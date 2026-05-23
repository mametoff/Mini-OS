; Простой тестовый loader

[BITS 16]
[ORG 0x1000]

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    sti

    mov si, msg
    call print
    jmp $

print:
    lodsb
    or al, al
    jz .done
    mov ah, 0x0E
    int 0x10
    jmp print
.done:
    ret

msg db 'LOADER OK!', 13, 10, 0

times 8192-($-$$) db 0
