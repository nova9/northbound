FROM debian:bookworm-slim AS downloader

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /download

COPY truenorthd/VERSION.txt ./VERSION.txt

RUN truenorth_version="$(cat VERSION.txt)" \
    && archive_url="https://github.com/truenorth-project/truenorth/releases/download/v${truenorth_version}/truenorth-v${truenorth_version}-linux-x86_64.tar.gz" \
    && curl --fail --location "$archive_url" --output truenorth.tar.gz \
    && tar --extract --gzip --file truenorth.tar.gz \
    && binary_directory="$(find . -type f -name truenorthd -path '*/bin/*' -exec dirname {} \; -quit)" \
    && test -n "$binary_directory" \
    && install "$binary_directory/truenorthd" /usr/local/bin/truenorthd \
    && install "$binary_directory/truenorth-cli" /usr/local/bin/truenorth-cli

FROM debian:trixie-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        libevent-core-2.1-7t64 \
        libevent-extra-2.1-7t64 \
        libevent-pthreads-2.1-7t64 \
        libnuma1 \
        libsqlite3-0 \
        libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 northbound \
    && mkdir --parents /var/lib/truenorth \
    && chown northbound:northbound /var/lib/truenorth

COPY --from=downloader /usr/local/bin/truenorthd /usr/local/bin/truenorthd
COPY --from=downloader /usr/local/bin/truenorth-cli /usr/local/bin/truenorth-cli
COPY truenorthd/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

RUN chmod +x /usr/local/bin/docker-entrypoint.sh

USER northbound

VOLUME ["/var/lib/truenorth"]

EXPOSE 8332 8333 18443 18444

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
