//! Os gatilhos de pressão: o kernel acorda o termostato.
//!
//! Em vez de olhar a pressão de memória a cada 2 s, o termostato se inscreve
//! no `/proc/pressure/memory` (PSI triggers, do kernel desde a 5.2): escreve
//! `some 150000 1000000` ("me avise quando, em qualquer janela de 1 s, algum
//! processo ficar 150 ms parado esperando memória") e espera num `poll`.
//!
//! A regra vai **com o terminador `\0`**: o kernel trata o último byte recebido
//! como fim de texto, e sem ele come o último dígito (`1000000` vira `100000`
//! e o kernel recusa com EINVAL). Foi o que aconteceu na primeira instalação,
//! em 19/11/2026. O
//! kernel o acorda em milissegundos, e o laço principal age na hora, sem
//! esperar a próxima volta. É o sinal que, em 19/11/2026, teria visto o
//! sufoco que travou a fedora.
//!
//! A espera não gasta CPU: a thread fica parada no `poll` até o kernel chamar.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rustix::event::{PollFd, PollFlags, poll};

/// O que a thread do gatilho conta ao laço.
#[derive(Default)]
pub struct Alarme {
    /// Há um aviso do kernel ainda não visto pelo laço.
    pub tocou: AtomicBool,
    /// Quantas vezes o kernel avisou desde a subida.
    pub vezes: AtomicU64,
}

/// O texto que o kernel entende: `some <parado µs> <janela µs>`.
pub fn regra(parado_ms: u64, janela_ms: u64) -> String {
    format!("some {} {}", parado_ms * 1000, janela_ms * 1000)
}

/// Espera no descritor até ele sinalizar `flags`, marcando o alarme a cada
/// vez. Só volta se o descritor der erro.
pub fn esperar<F: AsFd>(fd: &F, flags: PollFlags, alarme: &Alarme) -> Result<(), String> {
    loop {
        let mut fds = [PollFd::new(fd, flags)];
        poll(&mut fds, None).map_err(|e| format!("poll: {e}"))?;
        let r = fds[0].revents();
        if r.intersects(PollFlags::ERR | PollFlags::NVAL) {
            return Err("o kernel fechou o gatilho".into());
        }
        if r.intersects(flags) {
            alarme.vezes.fetch_add(1, Ordering::Relaxed);
            alarme.tocou.store(true, Ordering::Relaxed);
            if r.contains(PollFlags::HUP) {
                return Ok(());
            }
            // O PSI sinaliza no máximo uma vez por janela; dorme o resto para
            // não girar à toa com o mesmo aviso.
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

/// Inscreve o gatilho e deixa uma thread esperando. Sem PSI (kernel antigo,
/// testes no Mac), devolve o motivo e o termostato segue só com as leituras.
pub fn armar(proc: &Path, parado_ms: u64, janela_ms: u64) -> Result<Arc<Alarme>, String> {
    let caminho = proc.join("pressure/memory");
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&caminho)
        .map_err(|e| format!("{}: {e}", caminho.display()))?;
    f.write_all(format!("{}\0", regra(parado_ms, janela_ms)).as_bytes())
        .map_err(|e| format!("{}: o kernel recusou o gatilho: {e}", caminho.display()))?;
    let alarme = Arc::new(Alarme::default());
    let a = Arc::clone(&alarme);
    std::thread::spawn(move || {
        if let Err(e) = esperar(&f, PollFlags::PRI, &a) {
            eprintln!("⚠️ gatilho de pressão: {e}");
        }
    });
    Ok(alarme)
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    #[test]
    fn a_regra_vai_em_microssegundos() {
        assert_eq!(regra(150, 1000), "some 150000 1000000");
    }

    #[test]
    fn o_alarme_toca_quando_o_descritor_sinaliza() {
        // Um par de sockets faz o papel do PSI: escrever num acorda o poll do
        // outro (com POLLIN no lugar do POLLPRI do kernel).
        let (mut a, b) = UnixStream::pair().unwrap();
        let alarme = Arc::new(Alarme::default());
        let al = Arc::clone(&alarme);
        let t = std::thread::spawn(move || esperar(&b, PollFlags::IN, &al));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!alarme.tocou.load(Ordering::Relaxed));
        a.write_all(b"pressao").unwrap();
        for _ in 0..40 {
            if alarme.tocou.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(alarme.tocou.load(Ordering::Relaxed));
        assert_eq!(alarme.vezes.load(Ordering::Relaxed), 1);
        drop(a);
        let _ = t.join();
    }

    #[test]
    fn sem_psi_o_motivo_volta() {
        let d = tempfile::tempdir().unwrap();
        assert!(armar(d.path(), 150, 1000).is_err());
    }
}
