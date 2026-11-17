/* The kinds of socket a program run as an agent may open, written as a seccomp filter at build time
 * (modules/agent.nix): loaded by bubblewrap under `torq agent run` (--seccomp, modules/agent-run.sh) and by
 * torq-agent-scope in the agent's service (--families, agent/src/scope/main.rs), so both ways allow the same. The
 * families are what the service's RestrictAddressFamilies allows: unix, IPv4, IPv6 and netlink. Every other family is
 * refused as systemd refuses it (EAFNOSUPPORT), and `socketpair` of any family but unix, which systemd does not
 * judge: the packet filter's fence covers IPv4 and IPv6 only, and a socket to the hypervisor (AF_VSOCK), Bluetooth,
 * TIPC or RDS would be a way out past it (the security reader, 2026-11-17). Only this machine's own system call
 * ABI: a program of another (32-bit x86, whose socketcall hides the family from a filter) is ended, as the
 * service's SystemCallArchitectures=native ends it.
 *
 * Writes the filter (BPF) to its standard output. */
#include <errno.h>
#include <seccomp.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

static const int allowed[] = { AF_UNIX, AF_INET, AF_INET6, AF_NETLINK };

static int is_allowed(int family) {
    for (unsigned i = 0; i < sizeof allowed / sizeof allowed[0]; i++)
        if (allowed[i] == family)
            return 1;
    return 0;
}

int main(void) {
    scmp_filter_ctx ctx = seccomp_init(SCMP_ACT_ALLOW);
    if (ctx == NULL)
        return 1;
    int rc = 0;
    /* Each family below the last allowed one that is not allowed, and every family above it (the argument
     * compared as the whole register: anything with a bit set past the family's is above). */
    for (int family = 0; family < AF_NETLINK; family++)
        if (!is_allowed(family))
            rc |= seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EAFNOSUPPORT), SCMP_SYS(socket), 1, SCMP_A0(SCMP_CMP_EQ, family));
    rc |= seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EAFNOSUPPORT), SCMP_SYS(socket), 1, SCMP_A0(SCMP_CMP_GT, AF_NETLINK));
    /* A pair of sockets made at once (socketpair) of no family but unix: `socket` above is not the only call that
     * makes one, and systemd's RestrictAddressFamilies judges `socket` alone (src/shared/seccomp-util.c,
     * seccomp_restrict_address_families: its rules are of SCMP_SYS(socket) only). The kernel makes a pair for few
     * families (unix, TIPC); unix is the one a program needs (its children's pipes, Node's among them). */
    rc |= seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EAFNOSUPPORT), SCMP_SYS(socketpair), 1, SCMP_A0(SCMP_CMP_NE, AF_UNIX));
    /* And no io_uring: it makes a socket of any family by an operation of its own, never through the `socket`
     * call above (the controller's review, 2026-11-17). Refused as the agent's service refuses it (EPERM); a
     * program then does what it does where the kernel has none. */
    rc |= seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EPERM), SCMP_SYS(io_uring_setup), 0);
    rc |= seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EPERM), SCMP_SYS(io_uring_enter), 0);
    rc |= seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EPERM), SCMP_SYS(io_uring_register), 0);
    if (rc != 0 || seccomp_export_bpf(ctx, STDOUT_FILENO) != 0) {
        fprintf(stderr, "agent-socket-families: the filter could not be made\n");
        return 1;
    }
    seccomp_release(ctx);
    return 0;
}
