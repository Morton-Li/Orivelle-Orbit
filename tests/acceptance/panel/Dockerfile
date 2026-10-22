ARG PHP_BASE
FROM ${PHP_BASE}
LABEL codex.task="orbit-desktop-01a132f1" codex.scope="panel-acceptance"

# Temporary acceptance runtime only; application source is mounted separately.
RUN docker-php-ext-install -j2 bcmath pdo_mysql \
    && sed -i '/^user = /d; /^group = /d' /usr/local/etc/php-fpm.d/www.conf
COPY acceptance.ini /usr/local/etc/php/conf.d/zz-orbit-acceptance.ini
WORKDIR /app
