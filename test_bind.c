#include <stdio.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <errno.h>

int main() {
    int server_fd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr;
    addr.sin_family = AF_INET;
    addr.sin_port = htons(8080);
    addr.sin_addr.s_addr = INADDR_ANY;
    
    int res = bind(server_fd, (struct sockaddr *)&addr, sizeof(addr));
    printf("bind res: %d, errno: %d\n", res, errno);
    printf("sizeof sockaddr_in: %zu\n", sizeof(addr));
    printf("AF_INET: %d\n", AF_INET);
    printf("SOCK_STREAM: %d\n", SOCK_STREAM);
    return 0;
}
