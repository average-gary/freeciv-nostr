/***********************************************************************
 Freeciv - Copyright (C) 1996 - A Kjeldberg, L Gregersen, P Unold
   This program is free software; you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation; either version 2, or (at your option)
   any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.
***********************************************************************/

#ifdef HAVE_CONFIG_H
#include <fc_config.h>
#endif

#include "fc_prehdrs.h"

#include <errno.h>
#include <string.h>

#ifdef HAVE_SYS_TYPES_H
#include <sys/types.h>
#endif
#ifdef HAVE_SYS_SOCKET_H
#include <sys/socket.h>
#endif
#ifdef HAVE_NETINET_IN_H
#include <netinet/in.h>
#endif
#ifdef HAVE_NETDB_H
#include <netdb.h>
#endif
#ifdef HAVE_ARPA_INET_H
#include <arpa/inet.h>
#endif
#ifdef HAVE_SYS_SELECT_H
#include <sys/select.h>
#endif

/* utility */
#include "log.h"
#include "netintf.h"
#include "support.h"

/* common */
#include "fc_types.h"

#include "transport.h"

/* Verify FC_TRANSPORT_POLL_MAX is large enough for all connections
 * plus listen sockets. MAX_NUM_CONNECTIONS is defined in fc_types.h. */
FC_STATIC_ASSERT(FC_TRANSPORT_POLL_MAX > MAX_NUM_CONNECTIONS,
                 poll_max_must_exceed_max_connections);

/**********************************************************************//**
  TCP Backend Implementation

  This wraps the existing netintf.c socket functions to implement
  the fc_transport_ops interface. It is the default backend.
**************************************************************************/

/**********************************************************************//**
  TCP: Create a listening socket bound to bind_addr:port.

  TODO: This returns a single handle for the first bindable address.
  sernet.c's server_open_socket() binds ALL resolved addresses (enabling
  dual-stack IPv4+IPv6 via separate listen sockets). To support dual-stack
  through the transport layer, either listen_at needs to return multiple
  handles, or the caller must invoke it per-address. Deferred to Phase 1.1
  integration work.
**************************************************************************/
static int tcp_listen_at(fc_transport_handle *out,
                         const char *bind_addr, int port,
                         int backlog)
{
  struct fc_sockaddr_list *addrs;
  int sock = -1;

  addrs = net_lookup_service(bind_addr, port, FC_ADDR_ANY);
  if (addrs == nullptr) {
    return -1;
  }

  fc_sockaddr_list_iterate(addrs, paddr) {
    sock = socket(paddr->saddr.sa_family, SOCK_STREAM, 0);
    if (sock < 0) {
      continue;
    }

#ifndef FREECIV_HAVE_WINSOCK
    /* SO_REUSEADDR considered harmful on Windows, necessary otherwise */
    {
      int one = 1;

      if (setsockopt(sock, SOL_SOCKET, SO_REUSEADDR,
                     (const char *)&one, sizeof(one)) == -1) {
        log_error("setsockopt SO_REUSEADDR failed: %s",
                  fc_strerror(fc_get_errno()));
      }
    }
#endif /* FREECIV_HAVE_WINSOCK */

#ifdef FREECIV_IPV6_SUPPORT
    if (paddr->saddr.sa_family == AF_INET6) {
      int one = 1;

      if (setsockopt(sock, IPPROTO_IPV6, IPV6_V6ONLY,
                     (const char *)&one, sizeof(one)) == -1) {
        log_error("setsockopt IPV6_V6ONLY failed: %s",
                  fc_strerror(fc_get_errno()));
      }
    }
#endif /* FREECIV_IPV6_SUPPORT */

    if (bind(sock, &paddr->saddr, sockaddr_size(paddr)) == 0) {
      if (listen(sock, backlog) == 0) {
        fc_nonblock(sock);
        *out = sock;
        fc_sockaddr_list_destroy(addrs);
        return 0;
      }
    }

    fc_closesocket(sock);
    sock = -1;
  } fc_sockaddr_list_iterate_end;

  fc_sockaddr_list_destroy(addrs);
  return -1;
}

/**********************************************************************//**
  TCP: Accept an incoming connection on a listening socket.
**************************************************************************/
static int tcp_accept_conn(fc_transport_handle listen_h,
                           fc_transport_handle *out,
                           char *dst_host, int dst_host_len)
{
  union fc_sockaddr fromend;
  socklen_t fromlen = sizeof(fromend);
  int new_sock;

  new_sock = accept(listen_h, &fromend.saddr, &fromlen);
  if (new_sock < 0) {
    /* Not distinguishing would-block here: callers use poll() to check
     * readiness before calling accept, so EAGAIN should not normally
     * occur. Checking errno for EAGAIN/EWOULDBLOCK after raw accept()
     * would also be broken on Windows (no fc_accept() wrapper exists
     * to call set_socket_errno()). Match sernet.c's approach. */
    return -1;
  }

  if (dst_host != nullptr && dst_host_len > 0) {
    if (getnameinfo(&fromend.saddr, fromlen,
                    dst_host, dst_host_len,
                    nullptr, 0, NI_NUMERICHOST) != 0) {
      fc_snprintf(dst_host, dst_host_len, "unknown");
    }
  }

  fc_nonblock(new_sock);
  *out = new_sock;
  return 0;
}

/**********************************************************************//**
  TCP: Connect to a remote host:port.

  TODO: The EINPROGRESS check after fc_connect() implies non-blocking
  connect semantics, but the socket is not set to non-blocking before
  the connect call. Currently connect completes synchronously (matching
  client/clinet.c behavior). If async connect is needed in the future,
  fc_nonblock() should be called before fc_connect(). Deferred.
**************************************************************************/
static int tcp_connect_to(fc_transport_handle *out,
                          const char *host, int port)
{
  struct fc_sockaddr_list *addrs;
  int sock = -1;

  addrs = net_lookup_service(host, port, FC_ADDR_ANY);
  if (addrs == nullptr) {
    return -1;
  }

  fc_sockaddr_list_iterate(addrs, paddr) {
    sock = socket(paddr->saddr.sa_family, SOCK_STREAM, 0);
    if (sock < 0) {
      continue;
    }

    if (fc_connect(sock, &paddr->saddr, sockaddr_size(paddr)) == 0
        || errno == EINPROGRESS) {
      *out = sock;
      fc_sockaddr_list_destroy(addrs);
      return 0;
    }

    fc_closesocket(sock);
    sock = -1;
  } fc_sockaddr_list_iterate_end;

  fc_sockaddr_list_destroy(addrs);
  return -1;
}

/**********************************************************************//**
  TCP: Close a socket.
**************************************************************************/
static void tcp_close(fc_transport_handle h)
{
  if (h >= 0) {
    fc_closesocket(h);
  }
}

/**********************************************************************//**
  TCP: Read from a socket.
**************************************************************************/
static int tcp_read(fc_transport_handle h, void *buf, size_t len)
{
  return fc_readsocket(h, buf, len);
}

/**********************************************************************//**
  TCP: Write to a socket.
**************************************************************************/
static int tcp_write(fc_transport_handle h, const void *buf, size_t len)
{
  return fc_writesocket(h, buf, len);
}

/**********************************************************************//**
  TCP: Poll a set of handles for readiness.
  Maps the poll set to fd_set + select().
**************************************************************************/
static int tcp_poll(struct fc_transport_poll_set *set, int timeout_ms)
{
  fd_set readfds, writefds, exceptfds;
  fc_timeval tv;
  fc_timeval *tvp;
  int maxfd = -1;
  int i, ret, ready;

  FC_FD_ZERO(&readfds);
  FC_FD_ZERO(&writefds);
  FC_FD_ZERO(&exceptfds);

  for (i = 0; i < set->count; i++) {
    int fd = set->entries[i].handle;

    set->entries[i].returned_events = 0;

    if (fd < 0) {
      continue;
    }
    if (fd >= FD_SETSIZE) {
      log_error("transport: fd %d exceeds FD_SETSIZE %d", fd, FD_SETSIZE);
      continue;
    }

    if (set->entries[i].requested_events & FC_TRANSPORT_READ) {
      FD_SET(fd, &readfds);
    }
    if (set->entries[i].requested_events & FC_TRANSPORT_WRITE) {
      FD_SET(fd, &writefds);
    }
    FD_SET(fd, &exceptfds);

    if (fd > maxfd) {
      maxfd = fd;
    }
  }

  if (maxfd < 0) {
    /* No valid handles to monitor. Avoid unnecessary syscall. */
    return 0;
  }

  if (timeout_ms < 0) {
    tvp = nullptr;
  } else {
    tv.tv_sec = timeout_ms / 1000;
    tv.tv_usec = (timeout_ms % 1000) * 1000;
    tvp = &tv;
  }

  ret = fc_select(maxfd + 1, &readfds, &writefds, &exceptfds, tvp);
  if (ret < 0) {
    return -1;
  }
  if (ret == 0) {
    return 0;
  }

  ready = 0;
  for (i = 0; i < set->count; i++) {
    int fd = set->entries[i].handle;

    if (fd < 0 || fd >= FD_SETSIZE) {
      continue;
    }

    if (FD_ISSET(fd, &readfds)) {
      set->entries[i].returned_events |= FC_TRANSPORT_READ;
    }
    if (FD_ISSET(fd, &writefds)) {
      set->entries[i].returned_events |= FC_TRANSPORT_WRITE;
    }
    if (FD_ISSET(fd, &exceptfds)) {
      set->entries[i].returned_events |= FC_TRANSPORT_ERROR;
    }

    if (set->entries[i].returned_events != 0) {
      ready++;
    }
  }

  return ready;
}

/**********************************************************************//**
  TCP: Set a socket to non-blocking mode.
**************************************************************************/
static void tcp_set_nonblock(fc_transport_handle h)
{
  fc_nonblock(h);
}

/**********************************************************************//**
  TCP backend operations vtable.
**************************************************************************/
static const struct fc_transport_ops tcp_transport_ops = {
  .name = "tcp",
  .listen_at = tcp_listen_at,
  .accept_conn = tcp_accept_conn,
  .connect_to = tcp_connect_to,
  .close = tcp_close,
  .read = tcp_read,
  .write = tcp_write,
  .poll = tcp_poll,
  .set_nonblock = tcp_set_nonblock,
};

/**********************************************************************//**
  Global state: the currently active transport backend.
**************************************************************************/
static const struct fc_transport_ops *current_ops = nullptr;

/**********************************************************************//**
  Initialize the transport layer with the default TCP backend.
**************************************************************************/
void fc_transport_init(void)
{
  current_ops = &tcp_transport_ops;
  log_verbose("transport: initialized with backend '%s'", current_ops->name);
}

/**********************************************************************//**
  Shut down the transport layer.
**************************************************************************/
void fc_transport_done(void)
{
  log_verbose("transport: shutting down backend '%s'",
              current_ops != nullptr ? current_ops->name : "(none)");
  current_ops = nullptr;
}

/**********************************************************************//**
  Replace the current transport backend.
**************************************************************************/
void fc_transport_set_backend(const struct fc_transport_ops *ops)
{
  fc_assert_ret(ops != nullptr);
  fc_assert_ret(ops->name != nullptr);
  fc_assert_ret(ops->listen_at != nullptr);
  fc_assert_ret(ops->accept_conn != nullptr);
  fc_assert_ret(ops->connect_to != nullptr);
  fc_assert_ret(ops->close != nullptr);
  fc_assert_ret(ops->read != nullptr);
  fc_assert_ret(ops->write != nullptr);
  fc_assert_ret(ops->poll != nullptr);
  /* set_nonblock is optional — async backends (e.g., QUIC) may leave it
   * NULL. The dispatch wrapper fc_transport_set_nonblock() checks for this. */

  log_normal("transport: switching backend from '%s' to '%s'",
             current_ops != nullptr ? current_ops->name : "(none)",
             ops->name);
  current_ops = ops;
}

/**********************************************************************//**
  Get the current transport ops.
**************************************************************************/
const struct fc_transport_ops *fc_transport_get_ops(void)
{
  return current_ops;
}

/**********************************************************************//**
  Get the name of the current backend.
**************************************************************************/
const char *fc_transport_backend_name(void)
{
  if (current_ops == nullptr) {
    return "(uninitialized)";
  }
  return current_ops->name;
}

/*--- Convenience wrappers ---*/

/**********************************************************************//**
  Dispatch listen_at through current backend.
**************************************************************************/
int fc_transport_listen(fc_transport_handle *out,
                        const char *bind_addr, int port,
                        int backlog)
{
  fc_assert_ret_val(current_ops != nullptr, -1);
  return current_ops->listen_at(out, bind_addr, port, backlog);
}

/**********************************************************************//**
  Dispatch accept_conn through current backend.
**************************************************************************/
int fc_transport_accept(fc_transport_handle listen_h,
                        fc_transport_handle *out,
                        char *dst_host, int dst_host_len)
{
  fc_assert_ret_val(current_ops != nullptr, -1);
  return current_ops->accept_conn(listen_h, out, dst_host, dst_host_len);
}

/**********************************************************************//**
  Dispatch connect_to through current backend.
**************************************************************************/
int fc_transport_connect(fc_transport_handle *out,
                         const char *host, int port)
{
  fc_assert_ret_val(current_ops != nullptr, -1);
  return current_ops->connect_to(out, host, port);
}

/**********************************************************************//**
  Dispatch close through current backend.
**************************************************************************/
void fc_transport_close(fc_transport_handle h)
{
  if (current_ops != nullptr) {
    current_ops->close(h);
  }
}

/**********************************************************************//**
  Dispatch read through current backend.
**************************************************************************/
int fc_transport_read(fc_transport_handle h, void *buf, size_t len)
{
  fc_assert_ret_val(current_ops != nullptr, -1);
  return current_ops->read(h, buf, len);
}

/**********************************************************************//**
  Dispatch write through current backend.
**************************************************************************/
int fc_transport_write(fc_transport_handle h, const void *buf, size_t len)
{
  fc_assert_ret_val(current_ops != nullptr, -1);
  return current_ops->write(h, buf, len);
}

/**********************************************************************//**
  Dispatch poll through current backend.
**************************************************************************/
int fc_transport_poll(struct fc_transport_poll_set *set, int timeout_ms)
{
  fc_assert_ret_val(current_ops != nullptr, -1);
  return current_ops->poll(set, timeout_ms);
}

/**********************************************************************//**
  Dispatch set_nonblock through current backend.
**************************************************************************/
void fc_transport_set_nonblock(fc_transport_handle h)
{
  if (current_ops != nullptr && current_ops->set_nonblock != nullptr) {
    current_ops->set_nonblock(h);
  }
}
