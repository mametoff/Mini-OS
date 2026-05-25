For building kernel:
1) copy config to the root of source of kernel
2) Rename it to .config (starting with dot)
3) Execute:
make ARCH=x86_64 CROSS_COMPILE=x86_64-linux-gnu-gcc -j$(nproc) bzImage

Note: You must have x86_64-linux-gnu-gcc compiler already installed.
Otherwise, run: apt update && apt install x86_64-linux-gnu-gcc 
