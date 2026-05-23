; boot.asm - использует boot-info-table от genisoimage

[BITS 16]
[ORG 0x7C00]

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    sti

    mov [bootDrive], dl

    mov si, msgBoot
    call print_string

    ; genisoimage с -boot-info-table записывает LBA boot-образа
    ; по смещению 8 в загрузочном секторе (4 байта, little-endian)
    ; Это LBA текущего сектора (начала disk.img)
    mov eax, [0x7C08]            ; читаем LBA из boot-info-table
    
    ; Добавляем 1 чтобы получить LBA loader'а (сектор 1 внутри disk.img)
    inc eax
    
    mov dword [dap.lba], eax
    mov word [dap.count], 16     ; 16 секторов
    mov word [dap.segment], 0x1000
    mov word [dap.offset], 0x0000

    mov ah, 0x42
    mov dl, [bootDrive]
    mov si, dap
    int 0x13
    jc disk_error

    mov si, msgLoad
    call print_string

    ; Прыгаем на loader
    mov dl, [bootDrive]
    jmp 0x1000:0x0000

disk_error:
    mov si, msgErr
    call print_string
    mov al, ah
    call print_hex
    jmp $

print_string:
    lodsb
    or al, al
    jz .done
    mov ah, 0x0E
    int 0x10
    jmp print_string
.done:
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

bootDrive db 0
msgBoot db 'Boot', 0
msgLoad db ' OK', 13, 10, 0
msgErr  db ' Error ', 0

dap:
    .size    db 16
    .reserved db 0
    .count   dw 0
    .offset  dw 0
    .segment dw 0
    .lba     dq 0

times 510-($-$$) db 0
dw 0xAA55
