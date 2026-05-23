; loader.asm - отладочный

[BITS 16]
[ORG 0x1000]

start:
    ; Сразу выводим символ 'L' в видеопамять (0xB8000)
    ; Это сработает даже если все сегменты сломаны
    mov ax, 0xB800
    mov es, ax
    mov byte [es:0], 'L'
    mov byte [es:1], 0x0F
    mov byte [es:2], 'D'
    mov byte [es:3], 0x0F

    ; Теперь настроим сегменты
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    sti

    mov [boot_drive], dl

    ; Выводим через BIOS
    mov si, msg_loader
    call print

    ; Выводим значения LBA
    mov si, msg_lba
    call print

    mov eax, [kernel_lba]
    call print_eax

    mov si, msg_irfs
    call print

    mov eax, [irfs_lba]
    call print_eax

    ; Пробуем загрузить kernel
    mov si, msg_load
    call print

    mov eax, [kernel_lba]
    cmp eax, 0
    je .error

    mov cx, 1                  ; 1 сектор для пробы
    mov bx, 0x9000
    mov es, bx
    xor bx, bx
    call read_sectors
    jc .disk_error

    ; Проверяем, что загрузилось
    mov ax, 0x9000
    mov es, ax
    mov al, [es:0]
    call print_hex
    mov al, [es:1]
    call print_hex

    mov si, msg_ok
    call print

    jmp $

.error:
    mov si, msg_no_lba
    call print
    jmp $

.disk_error:
    mov si, msg_disk_err
    call print
    mov al, ah
    call print_hex
    jmp $

; --------------------------------------------------
read_sectors:
    pusha
    mov [dap.lba_low], eax
    mov [dap.count], cx
    mov [dap.segment], es
    mov [dap.offset], bx

    mov ah, 0x42
    mov dl, [boot_drive]
    mov si, dap
    int 0x13
    popa
    ret

print:
    lodsb
    or al, al
    jz .done
    mov ah, 0x0E
    int 0x10
    jmp print
.done:
    ret

print_eax:
    push eax
    shr eax, 24
    call print_hex
    pop eax
    push eax
    shr eax, 16
    call print_hex
    pop eax
    push ax
    shr al, 4
    call print_nibble
    pop ax
    and al, 0x0F
    call print_nibble
    ret

print_hex:
    push ax
    shr al, 4
    call print_nibble
    pop ax
    and al, 0x0F
    call print_nibble
    ret

print_nibble:
    cmp al, 10
    jl .digit
    add al, 'A' - 10
    jmp .out
.digit:
    add al, '0'
.out:
    mov ah, 0x0E
    int 0x10
    ret

; --------------------------------------------------
boot_drive db 0

kernel_lba    dd 0
kernel_sectors dw 0
irfs_lba      dd 0
irfs_sectors  dw 0
irfs_size     dd 0

dap:
    .size      db 16
    .reserved  db 0
    .count     dw 0
    .offset    dw 0
    .segment   dw 0
    .lba_low   dd 0
    .lba_high  dd 0

msg_loader  db 13, 10, 'Loader!', 13, 10, 0
msg_lba     db 'Kernel LBA: ', 0
msg_irfs    db ' IRFS LBA: ', 0
msg_load    db 13, 10, 'Loading...', 0
msg_ok      db ' OK', 13, 10, 0
msg_no_lba  db ' No LBA!', 13, 10, 0
msg_disk_err db ' DiskErr=', 0

times 8192-($-$$) db 0
