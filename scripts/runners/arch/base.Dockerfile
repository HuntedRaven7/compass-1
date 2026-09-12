FROM archlinux:latest

# The package list lives in install-deps.sh so CI jobs running directly on
# archlinux:latest install exactly what this image contains.
COPY install-deps.sh /tmp/install-deps.sh
RUN /tmp/install-deps.sh && rm /tmp/install-deps.sh
