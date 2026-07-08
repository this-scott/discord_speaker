#clearly not done yet but preemptive.

# deploying on a t4g.nano
# debating between ec2 and installing docker or ecs.
# ec2 is definitely simpler

# let's not include docker on our $3/mo instances because 
# sudo dnf install -y docker


# at some point this is going to run
sudo docker run -it --rm --name certbot \
    -v "/etc/letsencrypt:/etc/letsencrypt" \
    -v "/var/lib/letsencrypt:/var/lib/letsencrypt" \
    certbot/certbot certonly