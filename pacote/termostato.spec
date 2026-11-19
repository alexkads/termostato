# O pacote do termostato. Gerado pelo pacote/gerar-rpm.sh, que compila o
# binário de release antes e passa a versão; não é para rodar à mão.

Name:           termostato
Version:        %{versao}
Release:        1%{?dist}
Summary:        Guarda o calor e a memória de um MacBook com Fedora: freio da CPU com previsão, TCC no chip, processos e métricas
License:        LicenseRef-Proprietary
URL:            https://github.com/alexkads/termostato
Source0:        termostato
Source1:        termostato.service
Source2:        termostato.toml
Source3:        README.md
Source4:        CHANGELOG.md
ExclusiveArch:  x86_64

BuildRequires:  systemd-rpm-macros
Requires:       systemd
%{?systemd_requires}

%description
Serviço em Rust que cuida do calor de um MacBook com Fedora. A CPU nunca fica
solta: no dia a dia vale o limite normal de potência (RAPL PL1); quando a
temperatura sobe, ou quando a previsão mostra que vai subir, uma curva de freio
aperta o limite antes do pico. O turbo curto (PL2) continua livre. Também põe as
ventoinhas do applesmc no máximo e publica métricas para o Prometheus em
127.0.0.1:9101. Desde a 0.5 vê os processos e aprende quem esquenta (freio
antecipado), sente a pressão de memória com o gatilho do kernel, segura no
cgroup quem abusa (modo observar por padrão), grava o TCC offset no chip (a
CPU se freia sozinha a 92 °C) e avisa o Grafana de tudo. Tudo se ajusta em
/etc/termostato.toml.

%prep
# Nada a preparar: o binário já vem compilado.

%build
# Idem.

%install
install -Dpm 0755 %{SOURCE0} %{buildroot}%{_bindir}/termostato
install -Dpm 0644 %{SOURCE1} %{buildroot}%{_unitdir}/termostato.service
install -Dpm 0644 %{SOURCE2} %{buildroot}%{_sysconfdir}/termostato.toml
install -dm 0755 %{buildroot}%{_sharedstatedir}/termostato
install -Dpm 0644 %{SOURCE3} %{buildroot}%{_docdir}/%{name}/README.md
install -Dpm 0644 %{SOURCE4} %{buildroot}%{_docdir}/%{name}/CHANGELOG.md

%post
%systemd_post termostato.service
if [ $1 -eq 1 ]; then
    # Primeira instalação. O thermald disputa o mesmo PL1 (o único desvio do
    # padrão do Fedora, desfeito na remoção) e o perfil de energia volta ao
    # balanced, o padrão.
    systemctl disable --now thermald.service >/dev/null 2>&1 || :
    gdbus call --system --dest org.freedesktop.UPower.PowerProfiles \
        --object-path /org/freedesktop/UPower/PowerProfiles \
        --method org.freedesktop.DBus.Properties.Set \
        org.freedesktop.UPower.PowerProfiles ActiveProfile "<'balanced'>" >/dev/null 2>&1 || :
    systemctl enable --now termostato.service >/dev/null 2>&1 || :
fi

%preun
%systemd_preun termostato.service
if [ $1 -eq 0 ]; then
    # Remoção de verdade (nunca na atualização): parar deixou o limite normal;
    # só aqui a fábrica volta (PL1, TCC offset e pisos das ventoinhas), e o
    # thermald é religado.
    %{_bindir}/termostato --desinstalar %{_sysconfdir}/termostato.toml || :
    systemctl enable --now thermald.service >/dev/null 2>&1 || :
fi

%postun
# Na atualização, reinicia com o binário novo. É seguro: parar deixa o limite
# normal, nunca os 100 W de fábrica.
%systemd_postun_with_restart termostato.service

%files
%{_bindir}/termostato
%{_unitdir}/termostato.service
%config(noreplace) %{_sysconfdir}/termostato.toml
%dir %{_sharedstatedir}/termostato
%doc %{_docdir}/%{name}/README.md
%doc %{_docdir}/%{name}/CHANGELOG.md

%changelog
* Thu Nov 19 2026 Alex <alexkads@users.noreply.github.com> - 0.7.0-1
- termostato chave=valor muda parâmetros com o serviço rodando; CPU50C segura a CPU num alvo de temperatura

* Thu Nov 19 2026 Alex <alexkads@users.noreply.github.com> - 0.6.2-1
- Modo observar anuncia uma vez cada cgroup e o devolveria com a calma, em vez de repetir a cada leitura

* Thu Nov 19 2026 Alex <alexkads@users.noreply.github.com> - 0.6.1-1
- Avisos ao Grafana esperam a rede subir no boot em vez de se perder
- Primeiro pacote gerado no Fedora 45

* Thu Nov 19 2026 Alex <alexkads@users.noreply.github.com> - 0.6.0-1
- MegaBlaster: TCC offset no chip (a CPU se freia sozinha a 92 °C), gatilho de
  pressão de memória do kernel, estrangulamento, domínios do RAPL e sensores do Mac
- Superpoderes: processos, histórico negativo, freio antecipado, mãos no cgroup
  (modo observar), avisos ao Grafana, subida cedo no boot
- Curva de freio com previsão do pico; a CPU nunca fica solta
- Primeiro pacote RPM
