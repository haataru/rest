#include <stdio.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/epoll.h>

int main() {
    printf("O_NONBLOCK = %d\n", O_NONBLOCK);
    printf("EAGAIN = %d\n", EAGAIN);
    printf("EWOULDBLOCK = %d\n", EWOULDBLOCK);
    printf("EPOLLIN = %d\n", EPOLLIN);
    printf("EPOLLOUT = %d\n", EPOLLOUT);
    printf("EPOLLET = %u\n", EPOLLET);
    return 0;
}
